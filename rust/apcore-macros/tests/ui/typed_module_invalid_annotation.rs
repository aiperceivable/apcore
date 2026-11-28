#[apcore_macros::module(id = "demo.invalid", read_only = true)]
async fn declaration(input: ()) -> Result<(), ()> {
    Ok(input)
}

fn main() {}
