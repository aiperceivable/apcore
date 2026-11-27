use apcore::APCore;
use serde_json::json;
use std::time::Instant;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = APCore::new();
    if std::env::args().any(|argument| argument == "--startup") {
        return Ok(());
    }
    let schema = json!({
        "type": "object",
        "properties": {"value": {"type": "integer"}},
        "required": ["value"]
    });
    client.module(
        "bench.echo",
        "Echo an integer",
        schema.clone(),
        schema,
        None,
        Vec::new(),
        None,
        None,
        Vec::new(),
        None,
        |inputs, _context| Box::pin(async move { Ok(inputs) }),
    )?;
    for index in 0..100 {
        client
            .call("bench.echo", json!({"value": index}), None, None)
            .await?;
    }
    let mut samples = Vec::with_capacity(1000);
    for index in 0..1000 {
        let input = json!({"value": index});
        let start = Instant::now();
        let result = client.call("bench.echo", input, None, None).await?;
        samples.push(start.elapsed().as_nanos());
        assert_eq!(result["value"], index);
    }
    samples.sort_unstable();
    println!(
        "{{\"pipeline_p50_us\": {}, \"pipeline_p95_us\": {}}}",
        samples[499] as f64 / 1000.0,
        samples[949] as f64 / 1000.0,
    );
    Ok(())
}
