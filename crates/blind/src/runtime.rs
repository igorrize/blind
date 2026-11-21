use std::{future::Future, io};

use tokio::runtime::{Builder, Runtime};

pub fn single_threaded() -> io::Result<Runtime> {
    Builder::new_current_thread().enable_all().build()
}

pub fn block_on<F: Future>(future: F) -> io::Result<F::Output> {
    Ok(single_threaded()?.block_on(future))
}
