# Task 3: Built-in Handlers

Extract 3 existing hardcoded conditions into handler classes.

## Handlers
- `_IdentityTypesHandler` — check context.identity.type in allowed list
- `_RolesHandler` — check role overlap
- `_MaxCallDepthHandler` — check call_chain length <= value

## Auto-registration at module load
