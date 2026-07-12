use std::io::{self, Write};

use utoipa::OpenApi;
use web_server::openapi::ApiDoc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());

    serde_json::to_writer_pretty(&mut output, &ApiDoc::openapi())?;
    writeln!(output)?;

    Ok(())
}
