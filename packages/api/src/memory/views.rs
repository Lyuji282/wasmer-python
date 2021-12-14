use crate::{errors::to_py_err, wasmer_inner::wasmer};
use pyo3::{
    class::PyMappingProtocol,
    exceptions::{PyIndexError, PyRuntimeError, PyValueError},
    prelude::*,
    types::{PyAny, PyInt, PyLong, PySequence, PySlice},
};
use std::{cell::Cell, cmp::min, ops::Range};

macro_rules! memory_view {
    ($class_name:ident over $wasm_type:ty | $bytes_per_element:expr) => {
        /// Represents a read-and-write view over the data of a
        /// memory.
        ///
        /// It is built by the `Memory.uint8_view` and siblings getters.
        ///
        /// It implements the [Python mapping
        /// protocol][mapping-protocol], so it is possible to read and
        /// write bytes with a standard Python API.
        ///
        /// [mapping-protocol]: https://docs.python.org/3/c-api/mapping.html
        ///
        /// ## Example
        ///
        /// This is an example for the `Uint8Array` view, but it is
        /// the same for its siblings!
        ///
        /// ```py
        /// from wasmer import Store, Module, Instance, Uint8Array
        ///
        /// module = Module(Store(), open('tests/tests.wasm', 'rb').read())
        /// instance = Instance(module)
        /// exports = instance.exports
        ///
        /// pointer = exports.string()
        /// memory = exports.memory.uint8_view(offset=pointer)
        /// nth = 0
        /// string = ''
        ///
        /// while (0 != memory[nth]):
        ///     string += chr(memory[nth])
        ///     nth += 1
        ///
        /// assert string == 'Hello, World!'
        /// ```
        #[pyclass]
        pub struct $class_name {
            pub(crate) memory: wasmer::Memory,
            pub(crate) offset: usize,
        }

        #[pymethods]
        impl $class_name {
            /// Gets the number of bytes per element.
            #[getter]
            fn bytes_per_element(&self) -> u8 {
                $bytes_per_element
            }
        }

        #[pyproto]
        impl PyMappingProtocol for $class_name {
            /// Returns the length of the memory view.
            fn __len__(&self) -> PyResult<usize> {
                Ok(self.memory.view::<$wasm_type>()[self.offset..].len())
            }

            /// Returns one or more values from the memory view.
            ///
            /// The `index` can be either a slice or an integer.
            fn __getitem__(&self, index: &PyAny) -> PyResult<PyObject> {
                let view = self.memory.view::<$wasm_type>();
                let offset = self.offset;
                let range = if let Ok(slice) = index.cast_as::<PySlice>() {
                    let slice = slice.indices(view.len() as _)?;

                    if slice.start >= slice.stop {
                        return Err(to_py_err::<PyIndexError, _>(format!(
                            "Slice `{}:{}` cannot be empty",
                            slice.start, slice.stop
                        )));
                    } else if slice.step > 1 {
                        return Err(to_py_err::<PyIndexError, _>(format!(
                            "Slice must have a step of 1 for now; given {}",
                            slice.step
                        )));
                    }

                    (offset + slice.start as usize)..(min(offset + slice.stop as usize, view.len()))
                } else if let Ok(index) = index.extract::<isize>() {
                    if index < 0 {
                        return Err(to_py_err::<PyIndexError, _>(
                            "Out of bound: Index cannot be negative",
                        ));
                    }

                    let index = offset + index as usize;

                    #[allow(clippy::range_plus_one)]
                    // Writing `index..=index` makes Clippy happy but
                    // the type of this expression is
                    // `RangeInclusive`, when the type of `range` is
                    // `Range`.
                    {
                        index..index + 1
                    }
                } else {
                    return Err(to_py_err::<PyValueError, _>(
                        "Only integers and slices are valid to represent an index",
                    ));
                };

                if view.len() <= (range.end - 1) {
                    return Err(to_py_err::<PyIndexError, _>(format!(
                        "Out of bound: Maximum index {} is larger than the memory size {}",
                        range.end - 1,
                        view.len()
                    )));
                }

                let gil = Python::acquire_gil();
                let py = gil.python();

                if range.end - range.start == 1 {
                    Ok(view[range.start].get().into_py(py))
                } else {
                    Ok(view[range]
                        .iter()
                        .map(Cell::get)
                        .collect::<Vec<$wasm_type>>()
                        .into_py(py))
                }
            }

            /// Sets one or more values in the memory view.
            ///
            /// The `index` and `value` can only be of type slice and
            /// list, or integer and integer.
            fn __setitem__(&mut self, index: &PyAny, value: &PyAny) -> PyResult<()> {
                let offset = self.offset;
                let view = self.memory.view::<$wasm_type>();

                if let (Ok(slice), Ok(values)) = (
                    index.cast_as::<PySlice>().map_err(PyErr::from),
                    value
                        .cast_as::<PySequence>()
                        .map_err(PyErr::from)
                        .and_then(|sequence| sequence.list()),
                ) {
                    let slice = slice.indices(view.len() as _)?;

                    if slice.start >= slice.stop {
                        return Err(to_py_err::<PyIndexError, _>(format!(
                            "Slice `{}:{}` cannot be empty",
                            slice.start, slice.stop
                        )));
                    } else if slice.step < 1 {
                        return Err(to_py_err::<PyIndexError, _>(format!(
                            "Slice must have a positive step; given {}",
                            slice.step
                        )));
                    }

                    let iterator = Range {
                        start: slice.start,
                        end: slice.stop,
                    }
                    .step_by(slice.step as usize);

                    // Normally unreachable since the slice is bound
                    // to the size of the memory view.
                    if iterator.len() > view.len() {
                        return Err(to_py_err::<PyIndexError, _>(format!(
                            "Out of bound: The given key slice will write out of memory; memory length is {}, memory offset is {}, slice length is {}",
                            view.len(),
                            offset,
                            iterator.len()
                        )));
                    }

                    for (index, value) in iterator.zip(values.iter()) {
                        let index = index as usize;
                        let value = value.extract::<$wasm_type>()?;

                        view[offset + index].set(value);
                    }

                    Ok(())
                } else if let (Ok(index), Ok(value)) = (
                    index
                        .cast_as::<PyLong>()
                        .map_err(PyErr::from)
                        .and_then(|pylong| pylong.extract::<isize>()),
                    value
                        .cast_as::<PyInt>()
                        .map_err(PyErr::from)
                        .and_then(|pyint| pyint.extract::<$wasm_type>()),
                ) {
                    if index < 0 {
                        return Err(to_py_err::<PyIndexError, _>(
                            "Out of bound: Index cannot be negative",
                        ));
                    }

                    let index = index as usize;

                    if view.len() <= offset + index {
                        Err(to_py_err::<PyIndexError, _>(format!(
                            "Out of bound: Absolute index {} is larger than the memory size {}",
                            offset + index,
                            view.len()
                        )))
                    } else {
                        view[offset + index].set(value);

                        Ok(())
                    }
                } else {
                    Err(to_py_err::<PyRuntimeError, _>("When setting data to the memory view, the index and the value can only have the following types: Either `int` and `int`, or `slice` and `sequence`"))
                }
            }
        }
    };
}

memory_view!(Uint8Array over u8|1);
memory_view!(Int8Array over i8|1);
memory_view!(Uint16Array over u16|2);
memory_view!(Int16Array over i16|2);
memory_view!(Uint32Array over u32|4);
memory_view!(Int32Array over i32|4);
memory_view!(Uint64Array over u64|8);
memory_view!(Int64Array over i64|8);
memory_view!(Float32Array over f32|4);
memory_view!(Float64Array over f64|8);
