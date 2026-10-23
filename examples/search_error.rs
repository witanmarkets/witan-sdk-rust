//! cargo run --example search_error -- "npm error code ENEEDAUTH"
#[tokio::main]
async fn main() -> Result<(), witan_sdk::Error> {
    let error = std::env::args().nth(1).unwrap_or_else(|| "npm error code ENEEDAUTH".into());
    let c = witan_sdk::Client::new();
    let answer = c.search_error(&error, None).await?;
    for hit in &answer.results {
        println!("{:>3}  {}  {}", hit.score.unwrap_or(0.0), hit.id, hit.title);
    }
    if let Some(hit) = answer.results.first() {
        println!("\n{}", c.read(&hit.id).await?.body);
    }
    Ok(())
}
