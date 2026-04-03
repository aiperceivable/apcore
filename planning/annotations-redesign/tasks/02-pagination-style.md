# Task 2: Pagination Style

Change `pagination_style` from restricted enum/union to open string.

## Python
- `Literal["cursor", "offset", "page"]` -> `str`
- Default remains `"cursor"`

## TypeScript
- `'cursor' | 'offset' | 'page'` -> `string`
- Default remains `'cursor'`

## Tests
- Construct with `pagination_style="custom"`, no error
