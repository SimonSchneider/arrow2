use std::fmt::{Debug, Formatter, Result, Write};

use super::super::fmt::write_vec;
use super::super::Offset;
use super::BinaryArray;

pub fn write_value<O: Offset, W: Write>(array: &BinaryArray<O>, index: usize, f: &mut W) -> Result {
    let bytes = array.value(index);
    let writer = |f: &mut W, index| write!(f, "{}", bytes[index]);

    write_vec(f, writer, None, bytes.len(), "None", false)
}

impl<O: Offset> Debug for BinaryArray<O> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let writer = |f: &mut Formatter, index| write_value(self, index, f);

        let head = if O::is_large() {
            "LargeBinaryArray"
        } else {
            "BinaryArray"
        };
        write!(f, "{}", head)?;
        write_vec(f, writer, self.validity(), self.len(), "None", false)
    }
}
