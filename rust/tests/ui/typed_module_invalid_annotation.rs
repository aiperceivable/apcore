#[apcore::module(id = "demo.invalid", read_only = true)]
async fn declaration(input: ()) -> Result<(), apcore::ModuleError> {
    Ok(input)
}

fn main() {}
