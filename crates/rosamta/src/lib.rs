use clap::Parser;

#[derive(Parser)]
#[command(name = "rosamta", version)]
struct Cli {}

/// Entry point shared by the `rosamta` and `rsmt` binaries.
pub fn main() {
    Cli::parse();
}
