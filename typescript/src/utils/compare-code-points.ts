/** Compare strings by Unicode code point, matching the protocol's canonical order. */
export function compareCodePoints(left: string, right: string): number {
  const a = left[Symbol.iterator]();
  const b = right[Symbol.iterator]();
  while (true) {
    const nextA = a.next();
    const nextB = b.next();
    if (nextA.done || nextB.done) return Number(!!nextB.done) - Number(!!nextA.done);
    const difference = nextA.value.codePointAt(0)! - nextB.value.codePointAt(0)!;
    if (difference !== 0) return difference;
  }
}
