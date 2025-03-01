//! Array class

use crate::avm2::activation::Activation;
use crate::avm2::array::ArrayStorage;
use crate::avm2::class::Class;
use crate::avm2::error::{make_error_1125, range_error};
use crate::avm2::method::{Method, NativeMethodImpl};
use crate::avm2::object::{array_allocator, ArrayObject, FunctionObject, Object, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::QName;
use crate::string::AvmString;
use bitflags::bitflags;
use std::cmp::{min, Ordering};
use std::mem::swap;

// All of these methods will be defined as AS3-namespaced methods.
const PUBLIC_AS3_INSTANCE_METHODS: &[(&str, NativeMethodImpl)] = &[
    ("concat", concat),
    ("join", join),
    ("forEach", for_each),
    ("map", map),
    ("filter", filter),
    ("every", every),
    ("some", some),
    ("indexOf", index_of),
    ("lastIndexOf", last_index_of),
    ("pop", pop),
    ("push", push),
    ("insertAt", insert_at),
    ("removeAt", remove_at),
    ("reverse", reverse),
    ("shift", shift),
    ("unshift", unshift),
    ("slice", slice),
    ("splice", splice),
    ("sort", sort),
    ("sortOn", sort_on),
];

// All of these methods will be defined on the `Array` class prototype.
const PUBLIC_PROTO_METHODS: &[(&str, NativeMethodImpl)] = &[
    ("concat", concat),
    ("join", join),
    ("forEach", for_each),
    ("map", map),
    ("filter", filter),
    ("every", every),
    ("some", some),
    ("indexOf", index_of),
    ("lastIndexOf", last_index_of),
    ("pop", pop),
    ("push", push),
    ("removeAt", remove_at),
    ("reverse", reverse),
    ("shift", shift),
    ("toLocaleString", to_locale_string),
    ("toString", to_string),
    ("unshift", unshift),
    ("slice", slice),
    ("splice", splice),
    ("sort", sort),
    ("sortOn", sort_on),
];

/// Implements `Array`'s instance initializer.
pub fn instance_init<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        if args.len() == 1 {
            if let Some(expected_len) = args.get(0).filter(|v| v.is_number()).map(|v| v.as_f64()) {
                if expected_len < 0.0 || expected_len.is_nan() || expected_len.fract() != 0.0 {
                    return Err(Error::AvmError(range_error(
                        activation,
                        &format!(
                            "Error #1005: Array index is not a positive integer ({expected_len})"
                        ),
                        1005,
                    )?));
                }

                array.set_length(expected_len as usize);

                return Ok(Value::Undefined);
            }
        }

        for (i, arg) in args.iter().enumerate() {
            array.set(i, *arg);
        }
    }

    Ok(Value::Undefined)
}

pub fn class_call<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    activation
        .avm2()
        .classes()
        .array
        .construct(activation, args)
}

/// Implements `Array`'s class initializer.
pub fn class_init<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let scope = activation.create_scopechain();
    let gc_context = activation.gc();
    let this_class = this.as_class_object().unwrap();
    let array_proto = this_class.prototype();

    for (name, method) in PUBLIC_PROTO_METHODS {
        array_proto.set_string_property_local(
            *name,
            FunctionObject::from_method(
                activation,
                Method::from_builtin(*method, name, gc_context),
                scope,
                None,
                None,
                None,
            )
            .into(),
            activation,
        )?;
        array_proto.set_local_property_is_enumerable(gc_context, (*name).into(), false);
    }

    Ok(Value::Undefined)
}

/// Implements `Array.length`'s getter
pub fn length<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(array) = this.as_array_storage() {
        return Ok(array.length().into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.length`'s setter
pub fn set_length<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        let size = args
            .get(0)
            .unwrap_or(&Value::Undefined)
            .coerce_to_u32(activation)?;
        array.set_length(size as usize);
    }

    Ok(Value::Undefined)
}

/// Bundle an already-constructed `ArrayStorage` in an `Object`.
pub fn build_array<'gc>(
    activation: &mut Activation<'_, 'gc>,
    array: ArrayStorage<'gc>,
) -> Value<'gc> {
    ArrayObject::from_storage(activation, array).into()
}

/// Implements `Array.concat`
#[allow(clippy::map_clone)] //You can't clone `Option<Ref<T>>` without it
pub fn concat<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let mut base_array = this
        .as_array_storage()
        .map(|a| a.clone())
        .unwrap_or_else(|| ArrayStorage::new(0));

    for arg in args {
        if let Some(other_array) = arg
            .as_object()
            .as_ref()
            .and_then(|obj| obj.as_array_storage())
        {
            base_array.append(&other_array);
        } else {
            base_array.push(*arg);
        }
    }

    Ok(build_array(activation, base_array))
}

/// Resolves array holes.
pub fn resolve_array_hole<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    i: usize,
    item: Option<Value<'gc>>,
) -> Result<Value<'gc>, Error<'gc>> {
    if let Some(item) = item {
        return Ok(item);
    }

    if let Some(proto) = this.proto() {
        let proto = Value::from(proto);

        proto.get_public_property(
            AvmString::new_utf8(activation.gc(), i.to_string()),
            activation,
        )
    } else {
        Ok(Value::Undefined)
    }
}

pub fn join_inner<'gc, 'a, 'ctxt, C>(
    activation: &mut Activation<'a, 'gc>,
    this: Object<'gc>,
    args: &[Value<'gc>],
    mut conv: C,
) -> Result<Value<'gc>, Error<'gc>>
where
    C: for<'b> FnMut(Value<'gc>, &'b mut Activation<'a, 'gc>) -> Result<Value<'gc>, Error<'gc>>,
{
    let mut separator = args.get(0).cloned().unwrap_or(Value::Undefined);
    if separator == Value::Undefined {
        separator = ",".into();
    }

    if let Some(array) = this.as_array_storage() {
        let string_separator = separator.coerce_to_string(activation)?;
        let mut accum = Vec::with_capacity(array.length());

        for (i, item) in array.iter().enumerate() {
            let item = resolve_array_hole(activation, this, i, item)?;

            if matches!(item, Value::Undefined) || matches!(item, Value::Null) {
                accum.push(activation.strings().empty());
            } else {
                accum.push(conv(item, activation)?.coerce_to_string(activation)?);
            }
        }

        return Ok(AvmString::new(
            activation.gc(),
            crate::string::join(&accum, &string_separator),
        )
        .into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.join`
pub fn join<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    join_inner(activation, this, args, |v, _act| Ok(v))
}

/// Implements `Array.toString`
pub fn to_string<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    join_inner(activation, this, &[",".into()], |v, _act| Ok(v))
}

/// Implements `Array.toLocaleString`
pub fn to_locale_string<'gc>(
    act: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    join_inner(act, this, &[",".into()], |v, activation| {
        if matches!(v, Value::Null | Value::Undefined) {
            Ok(v)
        } else {
            v.call_public_property("toLocaleString", &[], activation)
        }
    })
}

/// An iterator that allows iterating over the contents of an array whilst also
/// executing user code.
///
/// Note that this does not actually implement `Iterator` as this struct needs
/// to share access to the activation with you. We can't claim your activation
/// and give it back in `next`, so we instead ask for it in `next`, which is
/// incompatible with the trait.
///
/// This technically works with Array-shaped, non-Array objects, since we
/// access arrays in this iterator the same way user code would. If it is
/// necessary to only work with Arrays, you must first check for array storage
/// before creating this iterator.
///
/// The primary purpose of `ArrayIter` is to maintain lock safety in the
/// presence of arbitrary user code. It is legal for, say, a method callback to
/// mutate the array under iteration. Normally, holding an `Iterator` on the
/// array while this happens would cause a panic; this code exists to prevent
/// that.
pub struct ArrayIter<'gc> {
    array_object: Object<'gc>,
    pub index: u32,
    pub rev_index: u32,
}

impl<'gc> ArrayIter<'gc> {
    /// Construct a new `ArrayIter`.
    pub fn new(
        activation: &mut Activation<'_, 'gc>,
        array_object: Object<'gc>,
    ) -> Result<Self, Error<'gc>> {
        Self::with_bounds(activation, array_object, 0, u32::MAX)
    }

    /// Construct a new `ArrayIter` that is bounded to a given range.
    pub fn with_bounds(
        activation: &mut Activation<'_, 'gc>,
        array_object: Object<'gc>,
        start_index: u32,
        end_index: u32,
    ) -> Result<Self, Error<'gc>> {
        let length = Value::from(array_object)
            .get_public_property("length", activation)?
            .coerce_to_u32(activation)?;

        Ok(Self {
            array_object,
            index: start_index.min(length),
            rev_index: end_index.saturating_add(1).min(length),
        })
    }

    /// Get the next item from the front of the array
    ///
    /// Since this isn't a real iterator, this comes pre-enumerated; it yields
    /// a pair of the index and then the value.
    pub fn next(
        &mut self,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Option<(u32, Value<'gc>)>, Error<'gc>> {
        if self.index < self.rev_index {
            let i = self.index;

            self.index += 1;

            let val = self.array_object.get_index_property(i as usize);

            let val = if let Some(storage) = self.array_object.as_vector_storage() {
                // Special case for Vector- it throws an error if trying to access
                // an element that was removed
                val.ok_or_else(|| make_error_1125(activation, i as usize, storage.length()))?
            } else {
                val.unwrap_or(Value::Undefined)
            };

            Ok(Some((i, val)))
        } else {
            Ok(None)
        }
    }

    /// Get the next item from the back of the array.
    ///
    /// Since this isn't a real iterator, this comes pre-enumerated; it yields
    /// a pair of the index and then the value.
    pub fn next_back(
        &mut self,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Option<(u32, Value<'gc>)>, Error<'gc>> {
        if self.index < self.rev_index {
            self.rev_index -= 1;

            let i = self.rev_index;

            let val = self.array_object.get_index_property(i as usize);

            let val = if let Some(storage) = self.array_object.as_vector_storage() {
                // Special case for Vector- it throws an error if trying to access
                // an element that was removed
                val.ok_or_else(|| make_error_1125(activation, i as usize, storage.length()))?
            } else {
                val.unwrap_or(Value::Undefined)
            };

            Ok(Some((i, val)))
        } else {
            Ok(None)
        }
    }
}

/// Implements `Array.forEach`
pub fn for_each<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let callback = args.get(0).cloned().unwrap_or(Value::Undefined);
    let receiver = args.get(1).cloned().unwrap_or(Value::Null);
    let mut iter = ArrayIter::new(activation, this)?;

    while let Some((i, item)) = iter.next(activation)? {
        callback.call(activation, receiver, &[item, i.into(), this.into()])?;
    }

    Ok(Value::Undefined)
}

/// Implements `Array.map`
pub fn map<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let callback = args.get(0).cloned().unwrap_or(Value::Undefined);
    let receiver = args.get(1).cloned().unwrap_or(Value::Null);
    let mut new_array = ArrayStorage::new(0);
    let mut iter = ArrayIter::new(activation, this)?;

    while let Some((i, item)) = iter.next(activation)? {
        let new_item = callback.call(activation, receiver, &[item, i.into(), this.into()])?;

        new_array.push(new_item);
    }

    Ok(build_array(activation, new_array))
}

/// Implements `Array.filter`
pub fn filter<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let callback = args.get(0).cloned().unwrap_or(Value::Undefined);
    let receiver = args.get(1).cloned().unwrap_or(Value::Null);
    let mut new_array = ArrayStorage::new(0);
    let mut iter = ArrayIter::new(activation, this)?;

    while let Some((i, item)) = iter.next(activation)? {
        let is_allowed = callback
            .call(activation, receiver, &[item, i.into(), this.into()])?
            .coerce_to_boolean();

        if is_allowed {
            new_array.push(item);
        }
    }

    Ok(build_array(activation, new_array))
}

/// Implements `Array.every`
pub fn every<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let callback = args.get(0).cloned().unwrap_or(Value::Undefined);
    let receiver = args.get(1).cloned().unwrap_or(Value::Null);
    let mut iter = ArrayIter::new(activation, this)?;

    while let Some((i, item)) = iter.next(activation)? {
        let result = callback
            .call(activation, receiver, &[item, i.into(), this.into()])?
            .coerce_to_boolean();

        if !result {
            return Ok(false.into());
        }
    }

    Ok(true.into())
}

/// Implements `Array.some`
pub fn some<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let callback = args.get(0).cloned().unwrap_or(Value::Undefined);
    let receiver = args.get(1).cloned().unwrap_or(Value::Null);
    let mut iter = ArrayIter::new(activation, this)?;

    while let Some((i, item)) = iter.next(activation)? {
        let result = callback
            .call(activation, receiver, &[item, i.into(), this.into()])?
            .coerce_to_boolean();

        if result {
            return Ok(true.into());
        }
    }

    Ok(false.into())
}

/// Implements `Array.indexOf`
pub fn index_of<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(array) = this.as_array_storage() {
        let search_val = args.get(0).cloned().unwrap_or(Value::Undefined);
        let from = args
            .get(1)
            .cloned()
            .unwrap_or_else(|| 0.into())
            .coerce_to_u32(activation)?;

        for (i, val) in array.iter().enumerate() {
            let val = resolve_array_hole(activation, this, i, val)?;
            if i >= from as usize && val == search_val {
                return Ok(i.into());
            }
        }

        return Ok((-1).into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.lastIndexOf`
pub fn last_index_of<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(array) = this.as_array_storage() {
        let search_val = args.get(0).cloned().unwrap_or(Value::Undefined);
        let from = args
            .get(1)
            .cloned()
            .unwrap_or_else(|| i32::MAX.into())
            .coerce_to_u32(activation)?;

        for (i, val) in array.iter().enumerate().rev() {
            let val = resolve_array_hole(activation, this, i, val)?;
            if i <= from as usize && val == search_val {
                return Ok(i.into());
            }
        }

        return Ok((-1).into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.pop`
pub fn pop<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        return Ok(array.pop());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.push`
pub fn push<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        for arg in args {
            array.push(*arg)
        }
        return Ok(array.length().into());
    }

    Ok(Value::Undefined)
}

pub fn reverse<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        let mut last_non_hole_index = None;
        for (i, val) in array.iter().enumerate() {
            if val.is_some() {
                last_non_hole_index = Some(i + 1);
            }
        }

        let mut new_array = ArrayStorage::new(0);

        for i in (0..last_non_hole_index.unwrap_or_else(|| array.length().saturating_sub(1))).rev()
        {
            if let Some(value) = array.get(i) {
                new_array.push(value)
            } else {
                new_array.push_hole()
            }
        }

        new_array.set_length(array.length());

        swap(&mut *array, &mut new_array);

        return Ok(this.into());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.shift`
pub fn shift<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    _args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        return Ok(array.shift());
    }

    Ok(Value::Undefined)
}

/// Implements `Array.unshift`
pub fn unshift<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        for arg in args.iter().rev() {
            array.unshift(*arg)
        }
        return Ok(array.length().into());
    }

    Ok(Value::Undefined)
}

/// Resolve a possibly-negative array index to something guaranteed to be positive.
pub fn resolve_index<'gc>(
    activation: &mut Activation<'_, 'gc>,
    index: Value<'gc>,
    length: usize,
) -> Result<usize, Error<'gc>> {
    let index = index.coerce_to_i32(activation)?;

    Ok(if index < 0 {
        let offset = index as isize;
        length.saturating_sub((-offset) as usize)
    } else {
        (index as usize).min(length)
    })
}

/// Implements `Array.slice`
pub fn slice<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let array_length = this.as_array_storage().map(|a| a.length());

    if let Some(array_length) = array_length {
        let actual_start = resolve_index(
            activation,
            args.get(0).cloned().unwrap_or_else(|| 0.into()),
            array_length,
        )?;
        let actual_end = resolve_index(
            activation,
            args.get(1).cloned().unwrap_or_else(|| 0xFFFFFF.into()),
            array_length,
        )?;
        let mut new_array = ArrayStorage::new(0);
        for i in actual_start..actual_end {
            if i >= array_length {
                break;
            }

            new_array.push(resolve_array_hole(
                activation,
                this,
                i,
                this.as_array_storage().unwrap().get(i),
            )?);
        }

        return Ok(build_array(activation, new_array));
    }

    Ok(Value::Undefined)
}

/// Implements `Array.splice`
pub fn splice<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let array_length = this.as_array_storage().map(|a| a.length());

    if let Some(array_length) = array_length {
        if let Some(start) = args.get(0).cloned() {
            let actual_start = resolve_index(activation, start, array_length)?;
            let delete_count = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| array_length.into())
                .coerce_to_i32(activation)?;

            let actual_end = min(array_length, actual_start + delete_count as usize);
            let args_slice = if args.len() > 2 {
                args[2..].iter().cloned()
            } else {
                [].iter().cloned()
            };

            let contents = this
                .as_array_storage()
                .map(|a| a.iter().collect::<Vec<Option<Value<'gc>>>>())
                .unwrap();

            let mut resolved = Vec::with_capacity(contents.len());
            for (i, v) in contents.iter().enumerate() {
                resolved.push(resolve_array_hole(activation, this, i, *v)?);
            }

            let removed = resolved
                .splice(actual_start..actual_end, args_slice)
                .collect::<Vec<Value<'gc>>>();
            let removed_array = ArrayStorage::from_args(&removed[..]);

            let mut resolved_array = ArrayStorage::from_args(&resolved[..]);

            if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
                swap(&mut *array, &mut resolved_array)
            }

            return Ok(build_array(activation, removed_array));
        }
    }

    Ok(Value::Undefined)
}

/// Insert an element into a specific position of an array.
pub fn insert_at<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    splice(
        activation,
        this,
        &[
            args.get(0).cloned().unwrap_or(0.into()),
            0.into(),
            args.get(1).cloned().unwrap_or(Value::Undefined),
        ],
    )
}

bitflags! {
    /// The array options that a given sort operation may use.
    ///
    /// These are provided as a number by the VM and converted into bitflags.
    #[derive(Clone, Copy)]
    pub struct SortOptions: u8 {
        /// Request case-insensitive string value sort.
        const CASE_INSENSITIVE     = 1 << 0;

        /// Reverse the order of sorting.
        const DESCENDING           = 1 << 1;

        /// Reject sorting on arrays with multiple equivalent values.
        const UNIQUE_SORT          = 1 << 2;

        /// Yield a list of indices rather than sorting the array in-place.
        const RETURN_INDEXED_ARRAY = 1 << 3;

        /// Request numeric value sort.
        const NUMERIC              = 1 << 4;
    }
}

/// Identity closure shim which exists purely to decorate closure types with
/// the HRTB necessary to accept an activation.
fn constrain<'a, 'gc, 'ctxt, F>(f: F) -> F
where
    F: FnMut(&mut Activation<'a, 'gc>, Value<'gc>, Value<'gc>) -> Result<Ordering, Error<'gc>>,
{
    f
}

/// Sort array storage.
///
/// This function expects its values to have been pre-enumerated and
/// pre-resolved. They will be sorted in-place. It is the caller's
/// responsibility to place the resulting half of the sorted array wherever.
///
/// This function will reverse the sort order if `Descending` sort is requested.
///
/// This function will return `false` in the event that the `UniqueSort`
/// constraint has been violated (`sort_func` returned `Ordering::Equal`). In
/// this case, you should cancel the in-place sorting operation and return 0 to
/// the caller. In the event that this function yields a runtime error, the
/// contents of the `values` array will be sorted in a random order.
fn sort_inner<'a, 'gc, 'ctxt, C>(
    activation: &mut Activation<'a, 'gc>,
    values: &mut [(usize, Value<'gc>)],
    options: SortOptions,
    mut sort_func: C,
) -> Result<bool, Error<'gc>>
where
    C: FnMut(&mut Activation<'a, 'gc>, Value<'gc>, Value<'gc>) -> Result<Ordering, Error<'gc>>,
{
    let mut unique_sort_satisfied = true;

    qsort(values, &mut |(_, a), (_, b)| {
        let unresolved_a = *a;
        let unresolved_b = *b;

        if matches!(unresolved_a, Value::Undefined) && matches!(unresolved_b, Value::Undefined) {
            unique_sort_satisfied = false;
            return Ok(Ordering::Equal);
        } else if matches!(unresolved_a, Value::Undefined) {
            return Ok(Ordering::Greater);
        } else if matches!(unresolved_b, Value::Undefined) {
            return Ok(Ordering::Less);
        }

        sort_func(activation, *a, *b).map(|cmp| {
            if cmp == Ordering::Equal {
                unique_sort_satisfied = false;
                Ordering::Equal
            } else if options.contains(SortOptions::DESCENDING) {
                cmp.reverse()
            } else {
                cmp
            }
        })
    })?;

    Ok(!options.contains(SortOptions::UNIQUE_SORT) || unique_sort_satisfied)
}

/// A port of the avmplus QuickSort implementation.
///
/// This differs from Rust's `slice::sort` in the following way:
/// - the comparison function is faillible and can return an error, short-circuiting the sort;
/// - the comparison function isn't required to define a valid total order: in such cases, the sort
///   will permute the slice arbitrarily, but won't return an error.
///
/// Original code: https://github.com/adobe/avmplus/blob/master/core/ArrayClass.cpp#L637
///
/// NOTE: this is `pub(super)` so it can be called by `vector::sort`.
pub(super) fn qsort<T, E>(
    slice: &mut [T],
    cmp: &mut impl FnMut(&T, &T) -> Result<Ordering, E>,
) -> Result<(), E> {
    match slice {
        // Empty and one-element slices are trivially sorted.
        [] | [_] => return Ok(()),
        // Fast-path for two elements.
        [a, b] => {
            if cmp(a, b)?.is_gt() {
                swap(a, b);
            }
            return Ok(());
        }
        // Fast-path for three elements.
        [a, b, c] => {
            if cmp(a, b)?.is_gt() {
                swap(a, b);
            }
            if cmp(b, c)?.is_gt() {
                swap(b, c);
                if cmp(a, b)?.is_gt() {
                    swap(a, b);
                }
            }
            return Ok(());
        }
        _ => (),
    }

    // Select the middle element of the slice as the pivot, and put it at the beginning.
    slice.swap(0, slice.len() / 2);

    // Order the elements (excluding the pivot) such that all elements lower
    // than the pivot come before all elements greater than the pivot.
    //
    // This is done by iterating from both ends, swapping greater elements with
    // lower ones along the way.
    let mut left = 0;
    let mut right = slice.len();
    loop {
        // Find an element greater than the pivot from the left.
        loop {
            left += 1;
            if left >= slice.len() || cmp(&slice[left], &slice[0])?.is_gt() {
                break;
            }
        }

        // Find an element lower than the pivot from the right.
        loop {
            right -= 1;
            if right == 0 || cmp(&slice[right], &slice[0])?.is_lt() {
                break;
            }
        }

        // Nothing left to swap, we are done.
        if right < left {
            break;
        }

        // Otherwise, swap left and right, and keep going.
        slice.swap(left, right);
    }

    // Put the pivot in its final position.
    slice.swap(0, right);

    // The elements are now ordered as follows:
    // [..right]: lower partition
    // [right..left]: middle partition (equal to pivot)
    // [left..]: higher partition

    // Recurse into both higher and lower partitions, with the smallest first.
    let (mut fst, mut snd) = slice.split_at_mut(left);
    fst = &mut fst[..right];
    if fst.len() >= snd.len() {
        swap(&mut fst, &mut snd);
    }
    qsort(fst, cmp)?;
    qsort(snd, cmp)
}

pub fn compare_string_case_sensitive<'gc>(
    activation: &mut Activation<'_, 'gc>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<Ordering, Error<'gc>> {
    let string_a = a.coerce_to_string(activation)?;
    let string_b = b.coerce_to_string(activation)?;

    Ok(string_a.cmp(&string_b))
}

pub fn compare_string_case_insensitive<'gc>(
    activation: &mut Activation<'_, 'gc>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<Ordering, Error<'gc>> {
    let string_a = a.coerce_to_string(activation)?;
    let string_b = b.coerce_to_string(activation)?;

    Ok(string_a.cmp_ignore_case(&string_b))
}

pub fn compare_numeric<'gc>(
    activation: &mut Activation<'_, 'gc>,
    a: Value<'gc>,
    b: Value<'gc>,
) -> Result<Ordering, Error<'gc>> {
    let num_a = a.coerce_to_number(activation)?;
    let num_b = b.coerce_to_number(activation)?;

    if num_a.is_nan() && num_b.is_nan() {
        Ok(Ordering::Equal)
    } else if num_a.is_nan() {
        Ok(Ordering::Greater)
    } else if num_b.is_nan() {
        Ok(Ordering::Less)
    } else {
        Ok(num_a.partial_cmp(&num_b).unwrap())
    }
}

/// Take a sorted set of values and produce the result requested by the caller.
fn sort_postprocess<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Object<'gc>,
    options: SortOptions,
    unique_satisfied: bool,
    values: Vec<(usize, Value<'gc>)>,
) -> Result<Value<'gc>, Error<'gc>> {
    if unique_satisfied {
        if options.contains(SortOptions::RETURN_INDEXED_ARRAY) {
            return Ok(build_array(
                activation,
                ArrayStorage::from_storage(
                    values.iter().map(|(i, _v)| Some((*i).into())).collect(),
                ),
            ));
        } else {
            if let Some(mut old_array) = this.as_array_storage_mut(activation.gc()) {
                let new_vec = values
                    .iter()
                    .map(|(src, v)| {
                        if let Some(old_value) = old_array.get(*src) {
                            Some(old_value)
                        } else if !matches!(v, Value::Undefined) {
                            Some(*v)
                        } else {
                            None
                        }
                    })
                    .collect();

                let mut new_array = ArrayStorage::from_storage(new_vec);

                swap(&mut *old_array, &mut new_array);
            }

            return Ok(this.into());
        }
    }

    Ok(0.into())
}

/// Given a value, extract its array values.
///
/// If the value is not an array, this function yields `None`.
fn extract_array_values<'gc>(
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<Option<Vec<Value<'gc>>>, Error<'gc>> {
    let object = value.as_object();
    let holey_vec = if let Some(object) = object {
        if let Some(field_array) = object.as_array_storage() {
            field_array.clone()
        } else {
            return Ok(None);
        }
    } else {
        return Ok(None);
    };

    let mut unholey_vec = Vec::with_capacity(holey_vec.length());
    for (i, v) in holey_vec.iter().enumerate() {
        unholey_vec.push(resolve_array_hole(activation, object.unwrap(), i, v)?);
    }

    Ok(Some(unholey_vec))
}

/// Impl `Array.sort`
pub fn sort<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    let (compare_fnc, options) = if args.len() > 1 {
        (
            Some(args.get(0).cloned().unwrap_or(Value::Undefined)),
            SortOptions::from_bits_truncate(
                args.get(1)
                    .cloned()
                    .unwrap_or_else(|| 0.into())
                    .coerce_to_u32(activation)? as u8,
            ),
        )
    } else {
        let arg = args.get(0).cloned().unwrap_or(Value::Undefined);
        if let Some(callable) = arg
            .as_object()
            .filter(|o| o.as_class_object().is_some() || o.as_function_object().is_some())
        {
            (Some(callable.into()), SortOptions::empty())
        } else {
            (
                None,
                SortOptions::from_bits_truncate(arg.coerce_to_u32(activation)? as u8),
            )
        }
    };

    let mut values = if let Some(values) = extract_array_values(activation, this.into())? {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| (i, *v))
            .collect::<Vec<(usize, Value<'gc>)>>()
    } else {
        return Ok(0.into());
    };

    let unique_satisfied = if let Some(v) = compare_fnc {
        sort_inner(
            activation,
            &mut values,
            options,
            constrain(|activation, a, b| {
                let order = v
                    .call(activation, this.into(), &[a, b])?
                    .coerce_to_number(activation)?;

                if order > 0.0 {
                    Ok(Ordering::Greater)
                } else if order < 0.0 {
                    Ok(Ordering::Less)
                } else {
                    Ok(Ordering::Equal)
                }
            }),
        )?
    } else if options.contains(SortOptions::NUMERIC) {
        sort_inner(activation, &mut values, options, compare_numeric)?
    } else if options.contains(SortOptions::CASE_INSENSITIVE) {
        sort_inner(
            activation,
            &mut values,
            options,
            compare_string_case_insensitive,
        )?
    } else {
        sort_inner(
            activation,
            &mut values,
            options,
            compare_string_case_sensitive,
        )?
    };

    sort_postprocess(activation, this, options, unique_satisfied, values)
}

/// Given a value, extract its array values.
///
/// If the value is not an array, it will be returned as if it was present in a
/// one-element array containing itself. This is intended for use with parsing
/// parameters which are optionally arrays.
fn extract_maybe_array_values<'gc>(
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<Vec<Value<'gc>>, Error<'gc>> {
    Ok(extract_array_values(activation, value)?.unwrap_or_else(|| vec![value]))
}

/// If called with sortOn(Array), yields vec of stringified elements.
/// If called with sortOn(String), yields this one string.
/// Otherwise, yields an empty vec.
fn extract_field_names<'gc>(
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<Vec<AvmString<'gc>>, Error<'gc>> {
    if let Some(values) = extract_array_values(activation, value)? {
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            out.push(value.coerce_to_string(activation)?);
        }
        Ok(out)
    } else if let Value::String(s) = value {
        Ok(vec![s])
    } else {
        Ok(vec![])
    }
}

/// Given a value, extract its array values and coerce them to SortOptions.
///
/// If the value is not an array, it will be returned as if it was present in a
/// one-element array containing itself. This is intended for use with parsing
/// parameters which are optionally arrays. The returned value will still be
/// coerced into a string in this case.
fn extract_maybe_array_sort_options<'gc>(
    activation: &mut Activation<'_, 'gc>,
    value: Value<'gc>,
) -> Result<Vec<SortOptions>, Error<'gc>> {
    let values = extract_maybe_array_values(activation, value)?;

    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(SortOptions::from_bits_truncate(
            value.coerce_to_u32(activation)? as u8,
        ));
    }
    Ok(out)
}

/// Impl `Array.sortOn`
pub fn sort_on<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(field_names_value) = args.get(0).cloned() {
        let field_names = extract_field_names(activation, field_names_value)?;
        let mut options = extract_maybe_array_sort_options(
            activation,
            args.get(1).cloned().unwrap_or_else(|| 0.into()),
        )?;

        let first_option = options.get(0).cloned().unwrap_or_else(SortOptions::empty)
            & (SortOptions::UNIQUE_SORT | SortOptions::RETURN_INDEXED_ARRAY);
        let mut values = if let Some(values) = extract_array_values(activation, this.into())? {
            values
                .iter()
                .enumerate()
                .map(|(i, v)| (i, *v))
                .collect::<Vec<(usize, Value<'gc>)>>()
        } else {
            return Ok(0.into());
        };

        if options.len() < field_names.len() {
            options.resize(
                field_names.len(),
                options.last().cloned().unwrap_or_else(SortOptions::empty),
            );
        }

        let unique_satisfied = sort_inner(
            activation,
            &mut values,
            first_option,
            constrain(|activation, a, b| {
                for (field_name, options) in field_names.iter().zip(options.iter()) {
                    // note: these are incorrect: pretty sure
                    // if the object is null/undefined or does not have the field,
                    // it's treated as if the field's value was undefined.
                    // TODO: verify this and fix it
                    let a_object = a.null_check(activation, None)?;
                    let a_field = a_object.get_public_property(*field_name, activation)?;

                    let b_object = b.null_check(activation, None)?;
                    let b_field = b_object.get_public_property(*field_name, activation)?;

                    let ord = if options.contains(SortOptions::NUMERIC) {
                        compare_numeric(activation, a_field, b_field)?
                    } else if options.contains(SortOptions::CASE_INSENSITIVE) {
                        compare_string_case_insensitive(activation, a_field, b_field)?
                    } else {
                        compare_string_case_sensitive(activation, a_field, b_field)?
                    };

                    if matches!(ord, Ordering::Equal) {
                        continue;
                    }

                    if options.contains(SortOptions::DESCENDING) {
                        return Ok(ord.reverse());
                    } else {
                        return Ok(ord);
                    }
                }

                Ok(Ordering::Equal)
            }),
        )?;

        return sort_postprocess(activation, this, first_option, unique_satisfied, values);
    }

    Ok(0.into())
}

/// Implements `Array.removeAt`
pub fn remove_at<'gc>(
    activation: &mut Activation<'_, 'gc>,
    this: Value<'gc>,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, Error<'gc>> {
    let this = this.as_object().unwrap();

    if let Some(mut array) = this.as_array_storage_mut(activation.gc()) {
        let index = args
            .get(0)
            .cloned()
            .unwrap_or(Value::Undefined)
            .coerce_to_i32(activation)?;

        return Ok(array.remove(index).unwrap_or(Value::Undefined));
    }

    Ok(Value::Undefined)
}

/// Construct `Array`'s class.
pub fn create_class<'gc>(activation: &mut Activation<'_, 'gc>) -> Class<'gc> {
    let mc = activation.gc();
    let class = Class::new(
        QName::new(activation.avm2().namespaces.public_all(), "Array"),
        Some(activation.avm2().class_defs().object),
        Method::from_builtin(instance_init, "<Array instance initializer>", mc),
        Method::from_builtin(class_init, "<Array class initializer>", mc),
        activation.avm2().class_defs().class,
        mc,
    );

    class.set_instance_allocator(mc, array_allocator);
    class.set_call_handler(
        mc,
        Method::from_builtin(class_call, "<Array call handler>", mc),
    );

    const PUBLIC_INSTANCE_PROPERTIES: &[(
        &str,
        Option<NativeMethodImpl>,
        Option<NativeMethodImpl>,
    )] = &[("length", Some(length), Some(set_length))];
    class.define_builtin_instance_properties(
        mc,
        activation.avm2().namespaces.public_all(),
        PUBLIC_INSTANCE_PROPERTIES,
    );

    class.define_builtin_instance_methods(
        mc,
        activation.avm2().namespaces.as3,
        PUBLIC_AS3_INSTANCE_METHODS,
    );

    const CONSTANTS_UINT: &[(&str, u32)] = &[
        (
            "CASEINSENSITIVE",
            SortOptions::CASE_INSENSITIVE.bits() as u32,
        ),
        ("DESCENDING", SortOptions::DESCENDING.bits() as u32),
        ("NUMERIC", SortOptions::NUMERIC.bits() as u32),
        (
            "RETURNINDEXEDARRAY",
            SortOptions::RETURN_INDEXED_ARRAY.bits() as u32,
        ),
        ("UNIQUESORT", SortOptions::UNIQUE_SORT.bits() as u32),
    ];
    class.define_constant_uint_class_traits(
        activation.avm2().namespaces.public_all(),
        CONSTANTS_UINT,
        activation,
    );

    const CONSTANTS_INT: &[(&str, i32)] = &[("length", 1)];
    class.define_constant_int_class_traits(
        activation.avm2().namespaces.public_all(),
        CONSTANTS_INT,
        activation,
    );

    class.mark_traits_loaded(activation.gc());
    class
        .init_vtable(activation.context)
        .expect("Native class's vtable should initialize");

    let c_class = class.c_class().expect("Class::new returns an i_class");

    c_class.mark_traits_loaded(activation.gc());
    c_class
        .init_vtable(activation.context)
        .expect("Native class's vtable should initialize");

    class
}
