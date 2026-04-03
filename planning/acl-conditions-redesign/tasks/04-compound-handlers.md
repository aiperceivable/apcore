# Task 4: Compound Handlers

## $or
- Receives `evaluate_fn` via constructor
- Value must be list of condition dicts
- Returns True if ANY sub-condition set passes

## $not
- Receives `evaluate_fn` via constructor
- Value must be a dict
- Returns negation of evaluating sub-conditions

## Auto-registration with `ACL._evaluate_conditions` as evaluate_fn
