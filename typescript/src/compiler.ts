import ts from 'typescript';

type Schema = boolean | { [key: string]: Json };
type Json = string | number | boolean | null | Json[] | { [key: string]: Json };

function schemaFromType(type: ts.Type, checker: ts.TypeChecker, location: ts.Node, output = false): Schema {
  const active = new Map<ts.Type, string>();
  const recursive = new Set<string>();
  const definitions: Record<string, Schema> = {};
  let nextId = 0;

  function convert(value: ts.Type): Schema {
    const flags = value.flags;
    if (flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown)) return {};
    if (flags & ts.TypeFlags.Never) return { not: {} };
    if (flags & ts.TypeFlags.Null) return { type: 'null' };
    if (flags & ts.TypeFlags.StringLiteral) return { const: (value as ts.StringLiteralType).value };
    if (flags & ts.TypeFlags.NumberLiteral) return { const: (value as ts.NumberLiteralType).value };
    if (flags & ts.TypeFlags.BooleanLike) {
      const name = checker.typeToString(value);
      return name === 'true' || name === 'false' ? { const: name === 'true' } : { type: 'boolean' };
    }
    if (flags & ts.TypeFlags.String) return { type: 'string' };
    if (flags & ts.TypeFlags.Number) return { type: 'number' };
    if (value.isUnion()) {
      return { anyOf: value.types.filter(member => !(member.flags & ts.TypeFlags.Undefined)).map(convert) };
    }
    if (value.isIntersection()) return { allOf: value.types.map(convert) };
    if (checker.isTupleType(value)) {
      const target = (value as ts.TupleTypeReference).target;
      const members = checker.getTypeArguments(value as ts.TypeReference);
      const rest = target.elementFlags.findIndex(flag => (flag & ts.ElementFlags.Variable) !== 0);
      if (rest >= 0 && rest !== members.length - 1) {
        throw new Error('A variadic tuple with trailing members requires an explicit schema');
      }
      const prefix = rest >= 0 ? members.slice(0, rest) : members;
      return {
        type: 'array',
        ...(prefix.length > 0 ? { prefixItems: prefix.map(convert) } : {}),
        minItems: target.minLength,
        items: rest >= 0 ? convert(members[rest]) : false,
      };
    }
    if (checker.isArrayType(value) || checker.isArrayLikeType(value) && value.getSymbol()?.name === 'ReadonlyArray') {
      return { type: 'array', items: convert(checker.getTypeArguments(value as ts.TypeReference)[0]) };
    }
    if (!(flags & (ts.TypeFlags.Object | ts.TypeFlags.NonPrimitive)) || value.getCallSignatures().length !== 0) {
      throw new Error(`Unsupported schema type ${checker.typeToString(value)}`);
    }
    const previousId = active.get(value);
    if (previousId !== undefined) {
      recursive.add(previousId);
      return { $ref: `#/$defs/${previousId}` };
    }
    const id = `type${++nextId}`;
    active.set(value, id);
    const properties = Object.fromEntries(value.getProperties().map(symbol => {
      const propertyType = checker.getTypeOfSymbolAtLocation(symbol, symbol.valueDeclaration ?? location);
      const property = convert(propertyType);
      const description = ts.displayPartsToString(symbol.getDocumentationComment(checker)).trim();
      return [symbol.name, description && typeof property === 'object' ? { ...property, description } : property];
    }));
    const required = value.getProperties()
      .filter(symbol => !(symbol.flags & ts.SymbolFlags.Optional))
      .map(symbol => symbol.name);
    const index = checker.getIndexTypeOfType(value, ts.IndexKind.String);
    const schema: Schema = {
      type: 'object',
      properties,
      ...(required.length > 0 ? { required } : {}),
      ...(index === undefined ? {} : { additionalProperties: convert(index) }),
    };
    active.delete(value);
    if (recursive.has(id)) definitions[id] = schema;
    return schema;
  }

  function normalizeOutput(value: ts.Type): Schema {
    if (value.isUnion()) return { anyOf: value.types.map(normalizeOutput) };
    if (value.flags & (ts.TypeFlags.Null | ts.TypeFlags.Undefined | ts.TypeFlags.Void)) {
      return { type: 'object', properties: {}, additionalProperties: false };
    }
    const schema = convert(value);
    if (typeof schema === 'object' && (schema.type === 'object' || value.isIntersection())) {
      return schema;
    }
    return {
      type: 'object',
      properties: { result: schema },
      required: ['result'],
      additionalProperties: false,
    };
  }

  const schema = output ? normalizeOutput(type) : convert(type);
  if (typeof schema === 'object' && Object.keys(definitions).length > 0) {
    return { ...schema, $defs: definitions };
  }
  return schema;
}

function propertyName(property: ts.ObjectLiteralElementLike): string | undefined {
  const name = property.name;
  return name && (ts.isIdentifier(name) || ts.isStringLiteral(name)) ? name.text : undefined;
}

function jsonExpression(factory: ts.NodeFactory, value: Json): ts.Expression {
  if (value === null) return factory.createNull();
  if (typeof value === 'boolean') return value ? factory.createTrue() : factory.createFalse();
  if (typeof value === 'string') return factory.createStringLiteral(value);
  if (typeof value === 'number') {
    return value < 0
      ? factory.createPrefixUnaryExpression(ts.SyntaxKind.MinusToken, factory.createNumericLiteral(-value))
      : factory.createNumericLiteral(value);
  }
  if (Array.isArray(value)) {
    return factory.createArrayLiteralExpression(value.map(item => jsonExpression(factory, item)));
  }
  return factory.createObjectLiteralExpression(Object.entries(value).map(([name, item]) =>
    factory.createPropertyAssignment(factory.createComputedPropertyName(factory.createStringLiteral(name)), jsonExpression(factory, item))));
}

function isModuleFactory(checker: ts.TypeChecker, expression: ts.Expression): boolean {
  if (ts.isPropertyAccessExpression(expression)) {
    return expression.name.text === 'module'
      && checker.getSymbolAtLocation(expression.expression)?.declarations?.some(decl =>
        ts.isNamespaceImport(decl)
        && ts.isStringLiteral(decl.parent.parent.moduleSpecifier)
        && decl.parent.parent.moduleSpecifier.text === 'apcore-js') === true;
  }
  return checker.getSymbolAtLocation(expression)?.declarations?.some(decl => {
    if (!ts.isImportSpecifier(decl) || (decl.propertyName ?? decl.name).text !== 'module') return false;
    const source = decl.parent.parent.parent.moduleSpecifier;
    return ts.isStringLiteral(source) && source.text === 'apcore-js';
  }) === true;
}

/**
 * Emit missing module schemas from the type-checked callable before TypeScript
 * erases parameter and return types. Pass this transformer to `Program.emit`.
 */
export function inferModuleSchemas(program: ts.Program): ts.TransformerFactory<ts.SourceFile> {
  const checker = program.getTypeChecker();
  return context => {
    const { factory } = context;
    const visit: ts.Visitor = node => {
      if (!ts.isCallExpression(node) || !isModuleFactory(checker, node.expression)) {
        return ts.visitEachChild(node, visit, context);
      }
      const options = node.arguments[0];
      if (!options || !ts.isObjectLiteralExpression(options)) return ts.visitEachChild(node, visit, context);
      const optionsType = checker.getTypeAtLocation(options);
      const hasSchema = (name: string) => {
        const property = optionsType.getProperty(name);
        return property !== undefined && !(property.flags & ts.SymbolFlags.Optional);
      };
      const hasInput = hasSchema('inputSchema');
      const hasOutput = hasSchema('outputSchema');
      if (hasInput && hasOutput) return ts.visitEachChild(node, visit, context);
      const execute = options.properties.find(property => propertyName(property) === 'execute');
      const callable = execute && ts.isPropertyAssignment(execute) ? execute.initializer : execute;
      if (callable === undefined) return node;
      const type = ts.isShorthandPropertyAssignment(callable)
        ? checker.getTypeOfSymbolAtLocation(checker.getShorthandAssignmentValueSymbol(callable)!, callable)
        : checker.getTypeAtLocation(callable);
      const signature = type.getCallSignatures()[0];
      const input = signature?.parameters[0];
      const added: ts.PropertyAssignment[] = [];
      const emit = (name: string, schema: Json) => {
        added.push(factory.createPropertyAssignment(name, jsonExpression(factory, schema)));
      };

      if (!hasInput) {
        const inputType = input && checker.getTypeOfSymbolAtLocation(input, callable);
        if (inputType && inputType.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown)) {
          throw new Error(`FUNC_MISSING_TYPE_HINT: ${input?.name}`);
        }
        emit('inputSchema', inputType ? schemaFromType(inputType, checker, callable) : { type: 'object' });
      }
      if (!hasOutput) {
        const outputType = signature && checker.getAwaitedType(checker.getReturnTypeOfSignature(signature));
        if (!outputType || outputType.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown)) {
          throw new Error('FUNC_MISSING_RETURN_TYPE: module execute');
        }
        emit('outputSchema', schemaFromType(outputType, checker, callable, true));
      }
      return factory.updateCallExpression(node, node.expression, node.typeArguments, [
        factory.updateObjectLiteralExpression(options, [...added, ...options.properties]),
      ]);
    };
    return source => ts.visitEachChild(source, visit, context);
  };
}
