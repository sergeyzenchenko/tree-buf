macro_rules! impl_fixed {
    ($(($size:expr, $namespace:ident)),+) => {
        $(mod $namespace {
            use crate::prelude::*;
            use std::mem::{MaybeUninit};
            // This transmute is a (seriously unsafe) version
            // of std::mem::transmute which allows unmatched sizes
            // This is needed here because of a compiler issue preventing
            // std::mem::transmute to be used: https://github.com/rust-lang/rust/issues/47966
            use transmute::transmute;

            #[cfg(feature = "encode")]
            impl<T: Encodable> Encodable for [T; $size] {
                type EncoderArray = ArrayEncoder<T::EncoderArray>;
                fn encode_root<O: EncodeOptions>(&self, stream: &mut EncoderStream<'_, O>) -> RootTypeId {
                    profile!("encode_root");
                    match self.len() {
                        0 => RootTypeId::Array0,
                        1 => {
                            stream.encode_with_id(|stream| (&self[0]).encode_root(stream));
                            RootTypeId::Array1
                        }
                        _ => {
                            // TODO: Seems kind of redundant to have both the array len,
                            // and the bytes len. Though, it's not for obvious reasons.
                            // Maybe sometimes we can infer from context. Eg: bool always
                            // requires the same number of bits per item
                            encode_usize(self.len(), stream);

                            // TODO: When there are types that are already
                            // primitive (eg: Vec<f64>) it doesn't make sense
                            // to buffer at this level. Specialization may
                            // be useful here.
                            //
                            // TODO: See below, and just call buffer on the vec
                            // and flush it!
                            let mut encoder = T::EncoderArray::default();
                            for item in self.iter() {
                                encoder.buffer(item);
                            }

                            stream.encode_with_id(|stream| encoder.flush(stream));

                            RootTypeId::ArrayN
                        }
                    }
                }
            }

            // Overly verbose because of `?` requiring `From` See also ec4fa3ba-def5-44eb-9065-e80b59530af6
            #[cfg(feature = "decode")]
            impl<T: Decodable + Sized> Decodable for [T; $size] where DecodeError : From<<T::DecoderArray as DecoderArray>::Error> {
                type DecoderArray = ArrayDecoder<T::DecoderArray>;
                fn decode(sticks: DynRootBranch<'_>, options: &impl DecodeOptions) -> DecodeResult<Self> {
                    profile!("Decodable::decode");
                    match sticks {
                        DynRootBranch::Array0 => {
                            if $size != 0 {
                                return Err(DecodeError::SchemaMismatch);
                            } else {
                                let data: [MaybeUninit<T>; $size] = unsafe {
                                    MaybeUninit::uninit().assume_init()
                                };
                                // Safety - we can only get here if size = 0, so
                                // it's an empty array of uninit and doesn't need to
                                // be initialized
                                Ok(unsafe { transmute(data) })
                            }
                        },
                        DynRootBranch::Array1(inner) => {
                            if $size != 1 {
                                return Err(DecodeError::SchemaMismatch);
                            }
                            let inner = T::decode(*inner, options)?;

                            let mut data: [MaybeUninit<T>; $size] = unsafe {
                                MaybeUninit::uninit().assume_init()
                            };
                            data[0] = MaybeUninit::new(inner);
                            // Safety: Verified that size = 1, so the only element has
                            // been initialized
                            Ok(unsafe { transmute(data) })
                        }
                        DynRootBranch::Array { len, values } => {
                            if len != $size {
                                return Err(DecodeError::SchemaMismatch);
                            }
                            let mut decoder = T::DecoderArray::new(values, options)?;
                            let mut data: [MaybeUninit<T>; $size] = unsafe {
                                MaybeUninit::uninit().assume_init()
                            };

                            for elem in &mut data[..] {
                                *elem = MaybeUninit::new(decoder.decode_next()?);
                            }

                            Ok(unsafe { transmute(data) })
                        }
                        _ => Err(DecodeError::SchemaMismatch),
                    }
                }
            }


            #[cfg(feature = "encode")]
            #[derive(Debug, Default)]
            pub struct ArrayEncoder<T> {
                values: T,
            }

            #[cfg(feature = "decode")]
            pub struct ArrayDecoder<T> {
                values: T,
            }

            #[cfg(feature = "encode")]
            impl<T: Encodable> EncoderArray<[T; $size]> for ArrayEncoder<T::EncoderArray> {
                fn buffer<'a, 'b: 'a>(&'a mut self, value: &'b [T; $size]) {
                    // TODO: Consider whether buffer should actually just
                    // do something non-flat, (like literally push the Vec<T> into another Vec<T>)
                    // and the flattening could happen later at flush time. This may reduce memory cost.
                    // Careful though.
                    // I feel though that somehow this outer buffer type
                    // could fix the specialization problem above for single-vec
                    // values.
                    for item in value.iter() {
                        self.values.buffer(item);
                    }
                }
                fn flush<O: EncodeOptions>(self, stream: &mut EncoderStream<'_, O>) -> ArrayTypeId {
                    profile!("flush");
                    let Self { values } = self;
                    encode_usize($size, stream);
                    if $size != 0 {
                        stream.encode_with_id(|stream| values.flush(stream));
                    }

                    ArrayTypeId::ArrayFixed
                }
            }

            #[cfg(feature = "decode")]
            impl<T: DecoderArray> DecoderArray for ArrayDecoder<T> {
                type Decode = [T::Decode; $size];
                type Error = T::Error;
                fn new(sticks: DynArrayBranch<'_>, options: &impl DecodeOptions) -> DecodeResult<Self> {
                    profile!("DecoderArray::new");

                    match sticks {
                        DynArrayBranch::ArrayFixed { len, values } => {
                            if len != $size {
                                return Err(DecodeError::SchemaMismatch);
                            }
                            let values = T::new(*values, options)?;
                            Ok(ArrayDecoder { values })
                        }
                        _ => Err(DecodeError::SchemaMismatch),
                    }
                }
                fn decode_next(&mut self) -> Result<Self::Decode, Self::Error> {
                    let mut data: [MaybeUninit<T::Decode>; $size] = unsafe {
                        MaybeUninit::uninit().assume_init()
                    };

                    for elem in &mut data[..] {
                        *elem = MaybeUninit::new(self.values.decode_next()?);
                    }

                    // Safety - all elements initialized in loop
                    Ok(unsafe { transmute(data) })
                }
            }

        })+
    };
}

impl_fixed!(
    // TODO: Re-add these, consider the changes that have to occur in DecoderArray.
    // It should be relatively simple, Add Eg: ArrayFixed0 variant
    //(0, _0),
    //(1, _1),
    (2, _2),
    (3, _3),
    (4, _4),
    (5, _5),
    (6, _6),
    (7, _7),
    (8, _8),
    (9, _9),
    (10, _10),
    (11, _11),
    (12, _12),
    (13, _13),
    (14, _14),
    (15, _15),
    (16, _16),
    (32, _32),
    (64, _64),
    (128, _128),
    (256, _256),
    (512, _512)
);
