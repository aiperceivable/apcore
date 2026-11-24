//! YAML binding example: format a date string.
//!
//! Demonstrates loading a module from a canonical YAML binding definition.
//! Pair with `format_date.binding.yaml` in the same directory.

use apcore::BindingLoader;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut loader = BindingLoader::new();

    let binding_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("bindings")
        .join("format_date.binding.yaml");

    loader.load_from_yaml(&binding_path)?;

    println!("Registered bindings:");
    for module_id in loader.list_bindings() {
        println!("  - {module_id}");
        if let Ok(entry) = loader.resolve(module_id) {
            println!("    target: {}", entry.target);
            if let Some(desc) = &entry.description {
                println!("    description: {desc}");
            }
        }
    }

    Ok(())
}
