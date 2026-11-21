use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!(
        "{}-exec {}: executor is not implemented yet",
        blind_core::NAME,
        blind_core::VERSION
    );
    ExitCode::from(2)
}
