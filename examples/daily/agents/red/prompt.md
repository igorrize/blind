Write tests for the behaviour the task asks for, before the code exists. They must fail now and pass once the
task is done.
You can write only test files: `<module>/tests.rs` next to a module, and integration tests in
`crates/<crate>/tests/`. When a module has no `tests.rs` yet, create it and say in `summary` that the module needs
the line `#[cfg(test)] mod tests;`: the writer adds it.
Test what the task asks for, not how it will be built: call the functions the task names.
Return in `tests` the full names of the tests you wrote, like `conversation::tests::copies_the_last_answer`, and
in `summary` what they cover and what the writer must know.
