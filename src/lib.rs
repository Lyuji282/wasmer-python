#[macro_use]
extern crate cpython;

mod instance;
mod value;

use instance::Instance;
use value::Value;

py_module_initializer!(libwasm, initlibwasm, PyInit_wasm, |python, module| {
    module.add(
        python,
        "__doc__",
        "This extension exposes an API to manipulate and execute WebAssembly binaries.",
    )?;
    module.add_class::<Instance>(python)?;
    module.add_class::<Value>(python)?;

    Ok(())
});
