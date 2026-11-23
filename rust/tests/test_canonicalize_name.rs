//! Public bare-name canonicalization contract (apcore-rust issue #40).

use apcore::{canonicalize_name, CanonicalNameError, CanonicalNameResult};
use serde_json::json;

fn assert_success(original: &str, canonical: &str) {
    let result = canonicalize_name(original);
    assert_eq!(result.original_name, original);
    assert_eq!(result.canonical_name.as_deref(), Some(canonical));
    assert_eq!(result.error, None);
}

fn assert_error(original: &str, error: CanonicalNameError) {
    let result = canonicalize_name(original);
    assert_eq!(result.original_name, original);
    assert_eq!(result.canonical_name, None);
    assert_eq!(result.error, Some(error));
}

#[test]
fn test_canonicalize_name_case_boundaries() {
    for (name, canonical) in [
        ("HTTPClient", "http_client"),
        ("HTMLParser", "html_parser"),
        ("getValue", "get_value"),
        ("log2Base", "log2_base"),
        ("SendEmail", "send_email"),
        ("system", "system"),
    ] {
        assert_success(name, canonical);
    }
}

#[test]
fn test_canonicalize_name_repairs_ascii_separators_only() {
    for (name, canonical) in [
        ("  --Send..Email!?\r\n", "send_email"),
        ("send-- \t::email", "send_email"),
        ("send_email__", "send_email__"),
        ("send__email", "send__email"),
        ("send-_Email", "send__email"),
        ("a/b\\c", "a_b_c"),
    ] {
        assert_success(name, canonical);
    }
}

#[test]
fn test_canonicalize_name_empty_and_invalid_starts() {
    for name in ["", " \r\n\t", "!!!"] {
        assert_error(name, CanonicalNameError::EmptyName);
    }
    for name in ["2fa", "7z", "_lead", "__", "-_lead!"] {
        assert_error(name, CanonicalNameError::InvalidStart);
    }
}

#[test]
fn test_canonicalize_name_rejects_non_ascii_before_trimming() {
    for name in ["café", "\u{a0}send\u{a0}", "ǅelta", "发送", "📨Send", "7é"] {
        assert_error(name, CanonicalNameError::NonAscii);
    }
}

#[test]
fn test_canonicalize_name_length_and_error_precedence() {
    assert_success(&"a".repeat(192), &"a".repeat(192));
    assert_error(&"a".repeat(193), CanonicalNameError::NameTooLong);
    assert_error(
        &format!("_{}", "a".repeat(192)),
        CanonicalNameError::InvalidStart,
    );
    assert_error(
        &format!("{}é", "a".repeat(193)),
        CanonicalNameError::NonAscii,
    );
    assert_error(&"aB".repeat(65), CanonicalNameError::NameTooLong);
}

#[test]
fn test_canonicalize_name_public_exports_and_serialization() {
    let result: apcore::utils::CanonicalNameResult = apcore::utils::canonicalize_name("SendEmail");
    let error: apcore::utils::CanonicalNameError = CanonicalNameError::InvalidStart;
    assert_eq!(error, CanonicalNameError::InvalidStart);
    assert_eq!(result, canonicalize_name("SendEmail"));
    for (input, error) in [
        ("", "empty_name"),
        ("é", "non_ascii"),
        ("2fa", "invalid_start"),
    ] {
        let result = canonicalize_name(input);
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(
            wire,
            json!({"original_name": input, "canonical_name": null, "error": error})
        );
        assert_eq!(
            serde_json::from_value::<CanonicalNameResult>(wire).unwrap(),
            result
        );
    }
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(
        wire,
        json!({"original_name": "SendEmail", "canonical_name": "send_email", "error": null})
    );
    assert_eq!(
        serde_json::from_value::<CanonicalNameResult>(wire).unwrap(),
        result
    );
    assert_eq!(
        serde_json::to_value(CanonicalNameError::NameTooLong).unwrap(),
        "name_too_long"
    );
    let _result_schema = schemars::schema_for!(CanonicalNameResult);
    let _error_schema = schemars::schema_for!(CanonicalNameError);
}

#[test]
fn test_canonicalize_name_does_not_change_module_id_normalization() {
    assert!(apcore::normalize_to_canonical_id("send-email", "python").is_err());
    assert_eq!(
        apcore::normalize_to_canonical_id("MyModule.SendEmail", "python").unwrap(),
        "my_module.send_email"
    );
    assert_eq!(
        apcore::normalize_to_canonical_id("MyModule::SendEmail", "rust").unwrap(),
        "my_module.send_email"
    );
    assert_success("MyModule.SendEmail", "my_module_send_email");
}

#[test]
fn test_canonicalize_name_shared_conformance_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            crate::conformance_env::find_fixtures_root().join("canonicalize_name.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let cases = fixture["test_cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let id = case["id"].as_str().unwrap();
        assert_eq!(
            case["input"].as_object().unwrap().len(),
            1,
            "{id}: unsupported input"
        );
        let input = case["input"]["name"].as_str().unwrap();
        let result = canonicalize_name(input);
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire, case["expected"], "{id}: all result fields must match");
        assert_eq!(
            result.original_name, input,
            "{id}: original input must survive"
        );
        assert_eq!(
            serde_json::from_value::<CanonicalNameResult>(wire).unwrap(),
            result,
            "{id}: round trip"
        );
    }
}
