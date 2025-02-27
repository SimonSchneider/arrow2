use std::sync::Arc;

use crate::{
    bitmap::Bitmap,
    datatypes::{DataType, Field},
    error::ArrowError,
};

use super::{new_empty_array, new_null_array, Array};

mod ffi;
pub(super) mod fmt;
mod iterator;

/// A [`StructArray`] is a nested [`Array`] with an optional validity representing
/// multiple [`Array`] with the same number of rows.
/// # Example
/// ```
/// use std::sync::Arc;
/// use arrow2::array::*;
/// use arrow2::datatypes::*;
/// let boolean = Arc::new(BooleanArray::from_slice(&[false, false, true, true])) as Arc<dyn Array>;
/// let int = Arc::new(Int32Array::from_slice(&[42, 28, 19, 31])) as Arc<dyn Array>;
///
/// let fields = vec![
///     Field::new("b", DataType::Boolean, false),
///     Field::new("c", DataType::Int32, false),
/// ];
///
/// let array = StructArray::new(DataType::Struct(fields), vec![boolean, int], None);
/// ```
#[derive(Clone)]
pub struct StructArray {
    data_type: DataType,
    values: Vec<Arc<dyn Array>>,
    validity: Option<Bitmap>,
}

impl StructArray {
    /// Returns a new [`StructArray`].
    /// # Errors
    /// This function errors iff:
    /// * `data_type`'s physical type is not [`crate::datatypes::PhysicalType::Struct`].
    /// * the children of `data_type` are empty
    /// * the values's len is different from children's length
    /// * any of the values's data type is different from its corresponding children' data type
    /// * any element of values has a different length than the first element
    /// * the validity's length is not equal to the length of the first element
    pub fn try_new(
        data_type: DataType,
        values: Vec<Arc<dyn Array>>,
        validity: Option<Bitmap>,
    ) -> Result<Self, ArrowError> {
        let fields = Self::try_get_fields(&data_type)?;
        if fields.is_empty() {
            return Err(ArrowError::oos(
                "A StructArray must contain at least one field",
            ));
        }
        if fields.len() != values.len() {
            return Err(ArrowError::oos(
                "A StructArray must a number of fields in its DataType equal to the number of child values",
            ));
        }

        fields
            .iter().map(|a| &a.data_type)
            .zip(values.iter().map(|a| a.data_type()))
            .enumerate()
            .try_for_each(|(index, (data_type, child))| {
                if data_type != child {
                    Err(ArrowError::oos(format!(
                        "The children DataTypes of a StructArray must equal the children data types. 
                         However, the field {index} has data type {data_type:?} but the value has data type {child:?}"
                    )))
                } else {
                    Ok(())
                }
            })?;

        let len = values[0].len();
        values
            .iter()
            .map(|a| a.len())
            .enumerate()
            .try_for_each(|(index, a_len)| {
                if a_len != len {
                    Err(ArrowError::oos(format!(
                        "The children DataTypes of a StructArray must equal the children data types.
                         However, the values {index} has a length of {a_len}, which is different from values 0, {len}."
                    )))
                } else {
                    Ok(())
                }
            })?;

        if validity
            .as_ref()
            .map_or(false, |validity| validity.len() != len)
        {
            return Err(ArrowError::oos(
                "The validity length of a StructArray must match its number of elements",
            ));
        }

        Ok(Self {
            data_type,
            values,
            validity,
        })
    }

    /// Returns a new [`StructArray`]
    /// # Panics
    /// This function panics iff:
    /// * `data_type`'s physical type is not [`crate::datatypes::PhysicalType::Struct`].
    /// * the children of `data_type` are empty
    /// * the values's len is different from children's length
    /// * any of the values's data type is different from its corresponding children' data type
    /// * any element of values has a different length than the first element
    /// * the validity's length is not equal to the length of the first element
    pub fn new(data_type: DataType, values: Vec<Arc<dyn Array>>, validity: Option<Bitmap>) -> Self {
        Self::try_new(data_type, values, validity).unwrap()
    }

    /// Alias for `new`
    pub fn from_data(
        data_type: DataType,
        values: Vec<Arc<dyn Array>>,
        validity: Option<Bitmap>,
    ) -> Self {
        Self::new(data_type, values, validity)
    }

    /// Creates an empty [`StructArray`].
    pub fn new_empty(data_type: DataType) -> Self {
        if let DataType::Struct(fields) = &data_type {
            let values = fields
                .iter()
                .map(|field| new_empty_array(field.data_type().clone()).into())
                .collect();
            Self::new(data_type, values, None)
        } else {
            panic!("StructArray must be initialized with DataType::Struct");
        }
    }

    /// Creates a null [`StructArray`] of length `length`.
    pub fn new_null(data_type: DataType, length: usize) -> Self {
        if let DataType::Struct(fields) = &data_type {
            let values = fields
                .iter()
                .map(|field| new_null_array(field.data_type().clone(), length).into())
                .collect();
            Self::new(data_type, values, Some(Bitmap::new_zeroed(length)))
        } else {
            panic!("StructArray must be initialized with DataType::Struct");
        }
    }
}

// must use
impl StructArray {
    /// Deconstructs the [`StructArray`] into its individual components.
    #[must_use]
    pub fn into_data(self) -> (Vec<Field>, Vec<Arc<dyn Array>>, Option<Bitmap>) {
        let Self {
            data_type,
            values,
            validity,
        } = self;
        let fields = if let DataType::Struct(fields) = data_type {
            fields
        } else {
            unreachable!()
        };
        (fields, values, validity)
    }

    /// Creates a new [`StructArray`] that is a slice of `self`.
    /// # Panics
    /// * `offset + length` must be smaller than `self.len()`.
    /// # Implementation
    /// This operation is `O(F)` where `F` is the number of fields.
    #[must_use]
    pub fn slice(&self, offset: usize, length: usize) -> Self {
        assert!(
            offset + length <= self.len(),
            "offset + length may not exceed length of array"
        );
        unsafe { self.slice_unchecked(offset, length) }
    }

    /// Creates a new [`StructArray`] that is a slice of `self`.
    /// # Implementation
    /// This operation is `O(F)` where `F` is the number of fields.
    /// # Safety
    /// The caller must ensure that `offset + length <= self.len()`.
    #[must_use]
    pub unsafe fn slice_unchecked(&self, offset: usize, length: usize) -> Self {
        let validity = self
            .validity
            .clone()
            .map(|x| x.slice_unchecked(offset, length));
        Self {
            data_type: self.data_type.clone(),
            values: self
                .values
                .iter()
                .map(|x| x.slice_unchecked(offset, length).into())
                .collect(),
            validity,
        }
    }

    /// Sets the validity bitmap on this [`StructArray`].
    /// # Panic
    /// This function panics iff `validity.len() != self.len()`.
    #[must_use]
    pub fn with_validity(&self, validity: Option<Bitmap>) -> Self {
        if matches!(&validity, Some(bitmap) if bitmap.len() != self.len()) {
            panic!("validity should be as least as large as the array")
        }
        let mut arr = self.clone();
        arr.validity = validity;
        arr
    }
}

// Accessors
impl StructArray {
    #[inline]
    fn len(&self) -> usize {
        self.values[0].len()
    }

    /// The optional validity.
    #[inline]
    pub fn validity(&self) -> Option<&Bitmap> {
        self.validity.as_ref()
    }

    /// Returns the values of this [`StructArray`].
    pub fn values(&self) -> &[Arc<dyn Array>] {
        &self.values
    }

    /// Returns the fields of this [`StructArray`].
    pub fn fields(&self) -> &[Field] {
        Self::get_fields(&self.data_type)
    }
}

impl StructArray {
    /// Returns the fields the `DataType::Struct`.
    pub(crate) fn try_get_fields(data_type: &DataType) -> Result<&[Field], ArrowError> {
        match data_type.to_logical_type() {
            DataType::Struct(fields) => Ok(fields),
            _ => Err(ArrowError::oos(
                "Struct array must be created with a DataType whose physical type is Struct",
            )),
        }
    }

    /// Returns the fields the `DataType::Struct`.
    pub fn get_fields(data_type: &DataType) -> &[Field] {
        Self::try_get_fields(data_type).unwrap()
    }
}

impl Array for StructArray {
    #[inline]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[inline]
    fn len(&self) -> usize {
        self.len()
    }

    #[inline]
    fn data_type(&self) -> &DataType {
        &self.data_type
    }

    #[inline]
    fn validity(&self) -> Option<&Bitmap> {
        self.validity.as_ref()
    }

    fn slice(&self, offset: usize, length: usize) -> Box<dyn Array> {
        Box::new(self.slice(offset, length))
    }
    unsafe fn slice_unchecked(&self, offset: usize, length: usize) -> Box<dyn Array> {
        Box::new(self.slice_unchecked(offset, length))
    }
    fn with_validity(&self, validity: Option<Bitmap>) -> Box<dyn Array> {
        Box::new(self.with_validity(validity))
    }
    fn to_boxed(&self) -> Box<dyn Array> {
        Box::new(self.clone())
    }
}
