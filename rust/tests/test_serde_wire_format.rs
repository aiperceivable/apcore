// Regression tests: enum wire-format serialization (D2-001)
//
// Each test verifies that a Rust enum variant serializes to the exact JSON string
// expected by the Python and TypeScript SDKs on the wire.

use apcore::registry::conflicts::{ConflictSeverity, ConflictType};
use apcore::schema::exporter::ExportProfile;
use apcore::schema::loader::SchemaStrategy;

// ---------------------------------------------------------------------------
// ConflictType — snake_case wire format
// ---------------------------------------------------------------------------

#[test]
fn conflict_type_duplicate_id_serializes_to_snake_case() {
    let json = serde_json::to_string(&ConflictType::DuplicateId).unwrap();
    assert_eq!(json, r#""duplicate_id""#);
}

#[test]
fn conflict_type_reserved_word_serializes_to_snake_case() {
    let json = serde_json::to_string(&ConflictType::ReservedWord).unwrap();
    assert_eq!(json, r#""reserved_word""#);
}

#[test]
fn conflict_type_case_collision_serializes_to_snake_case() {
    let json = serde_json::to_string(&ConflictType::CaseCollision).unwrap();
    assert_eq!(json, r#""case_collision""#);
}

#[test]
fn conflict_type_deserializes_from_snake_case() {
    let value: ConflictType = serde_json::from_str(r#""duplicate_id""#).unwrap();
    assert_eq!(value, ConflictType::DuplicateId);

    let value: ConflictType = serde_json::from_str(r#""reserved_word""#).unwrap();
    assert_eq!(value, ConflictType::ReservedWord);

    let value: ConflictType = serde_json::from_str(r#""case_collision""#).unwrap();
    assert_eq!(value, ConflictType::CaseCollision);
}

// ---------------------------------------------------------------------------
// ConflictSeverity — snake_case wire format
// ---------------------------------------------------------------------------

#[test]
fn conflict_severity_error_serializes_to_snake_case() {
    let json = serde_json::to_string(&ConflictSeverity::Error).unwrap();
    assert_eq!(json, r#""error""#);
}

#[test]
fn conflict_severity_warning_serializes_to_snake_case() {
    let json = serde_json::to_string(&ConflictSeverity::Warning).unwrap();
    assert_eq!(json, r#""warning""#);
}

#[test]
fn conflict_severity_deserializes_from_snake_case() {
    let value: ConflictSeverity = serde_json::from_str(r#""error""#).unwrap();
    assert_eq!(value, ConflictSeverity::Error);

    let value: ConflictSeverity = serde_json::from_str(r#""warning""#).unwrap();
    assert_eq!(value, ConflictSeverity::Warning);
}

// ---------------------------------------------------------------------------
// TaskStatus — already had snake_case; verify it remains correct
// ---------------------------------------------------------------------------

#[test]
fn task_status_pending_serializes_to_snake_case() {
    use apcore::async_task::TaskStatus;
    let json = serde_json::to_string(&TaskStatus::Pending).unwrap();
    assert_eq!(json, r#""pending""#);
}

#[test]
fn task_status_running_serializes_to_snake_case() {
    use apcore::async_task::TaskStatus;
    let json = serde_json::to_string(&TaskStatus::Running).unwrap();
    assert_eq!(json, r#""running""#);
}

#[test]
fn task_status_completed_serializes_to_snake_case() {
    use apcore::async_task::TaskStatus;
    let json = serde_json::to_string(&TaskStatus::Completed).unwrap();
    assert_eq!(json, r#""completed""#);
}

#[test]
fn task_status_failed_serializes_to_snake_case() {
    use apcore::async_task::TaskStatus;
    let json = serde_json::to_string(&TaskStatus::Failed).unwrap();
    assert_eq!(json, r#""failed""#);
}

#[test]
fn task_status_cancelled_serializes_to_snake_case() {
    use apcore::async_task::TaskStatus;
    let json = serde_json::to_string(&TaskStatus::Cancelled).unwrap();
    assert_eq!(json, r#""cancelled""#);
}

// ---------------------------------------------------------------------------
// ExportProfile — snake_case wire format (matches Python: "mcp", "openai", etc.)
// ---------------------------------------------------------------------------

#[test]
fn export_profile_mcp_serializes_to_snake_case() {
    let json = serde_json::to_string(&ExportProfile::Mcp).unwrap();
    assert_eq!(json, r#""mcp""#);
}

#[test]
fn export_profile_openai_serializes_to_openai() {
    // Python ExportProfile.OPENAI = "openai" — explicit rename overrides snake_case
    let json = serde_json::to_string(&ExportProfile::OpenAi).unwrap();
    assert_eq!(json, r#""openai""#);
}

#[test]
fn export_profile_anthropic_serializes_to_snake_case() {
    let json = serde_json::to_string(&ExportProfile::Anthropic).unwrap();
    assert_eq!(json, r#""anthropic""#);
}

#[test]
fn export_profile_generic_serializes_to_snake_case() {
    let json = serde_json::to_string(&ExportProfile::Generic).unwrap();
    assert_eq!(json, r#""generic""#);
}

// ---------------------------------------------------------------------------
// SchemaStrategy — snake_case wire format (matches Python: "yaml_first", etc.)
// ---------------------------------------------------------------------------

#[test]
fn schema_strategy_yaml_first_serializes_to_snake_case() {
    let json = serde_json::to_string(&SchemaStrategy::YamlFirst).unwrap();
    assert_eq!(json, r#""yaml_first""#);
}

#[test]
fn schema_strategy_native_first_serializes_to_snake_case() {
    let json = serde_json::to_string(&SchemaStrategy::NativeFirst).unwrap();
    assert_eq!(json, r#""native_first""#);
}

#[test]
fn schema_strategy_yaml_only_serializes_to_snake_case() {
    let json = serde_json::to_string(&SchemaStrategy::YamlOnly).unwrap();
    assert_eq!(json, r#""yaml_only""#);
}

#[test]
fn schema_strategy_deserializes_from_snake_case() {
    let value: SchemaStrategy = serde_json::from_str(r#""yaml_first""#).unwrap();
    assert_eq!(value, SchemaStrategy::YamlFirst);

    let value: SchemaStrategy = serde_json::from_str(r#""native_first""#).unwrap();
    assert_eq!(value, SchemaStrategy::NativeFirst);

    let value: SchemaStrategy = serde_json::from_str(r#""yaml_only""#).unwrap();
    assert_eq!(value, SchemaStrategy::YamlOnly);
}

// ---------------------------------------------------------------------------
// ConflictResult round-trip
// ---------------------------------------------------------------------------

#[test]
fn conflict_result_round_trips_via_json() {
    use apcore::registry::conflicts::ConflictResult;

    let original = ConflictResult {
        conflict_type: ConflictType::DuplicateId,
        severity: ConflictSeverity::Error,
        message: "Module ID 'foo.bar' is already registered".to_string(),
    };

    let json = serde_json::to_string(&original).unwrap();
    assert!(
        json.contains(r#""duplicate_id""#),
        "expected snake_case conflict_type in JSON"
    );
    assert!(
        json.contains(r#""error""#),
        "expected snake_case severity in JSON"
    );

    let restored: ConflictResult = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, original);
}
