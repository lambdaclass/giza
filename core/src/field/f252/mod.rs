//! An implementation of the 252-bit STARK-friendly prime field chosen by Starkware
//! with modulus $2^{251} + 17 \cdot 2^{192} + 1$.
//! TODO: Worth switching to Barrett reduction for efficiency?
use core::{
    convert::{TryFrom, TryInto},
    fmt::{Debug, Display, Formatter, LowerHex},
    ops::{
        Add, AddAssign, BitAnd, Div, DivAssign, Mul, MulAssign, Neg, Shl, Shr, ShrAssign, Sub,
        SubAssign,
    },
    slice,
};
pub use math::{ExtensibleField, FieldElement, StarkField};
use winter_utils::{
    collections::Vec, string::String, AsBytes, ByteReader, ByteWriter, Deserializable,
    DeserializationError, Randomizable, Serializable,
};

use ff::{Field, PrimeField};

#[macro_use]
use serde::{Serialize, Deserialize};
use serde_hex::{SerHex, Strict};

// FIELD ELEMENT
// ================================================================================================

// Note that the internal representation of Fr is assumed to be in Montgomery form with R=2^256
#[derive(PrimeField, Serialize, Deserialize)]
#[PrimeFieldModulus = "3618502788666131213697322783095070105623107215331596699973092056135872020481"]
#[PrimeFieldGenerator = "3"]
#[PrimeFieldReprEndianness = "little"]
struct Fr(#[serde(with = "SerHex::<Strict>")] [u64; 4]);

// Number of bytes needed to represent field element
const ELEMENT_BYTES: usize = core::mem::size_of::<Fr>();

// A wrapper around the internal representation of Fr for non-finite field integer manipulation
#[derive(PartialOrd, Ord, PartialEq, Eq, Copy, Clone, Debug)]
pub struct BigInt(pub [u64; 4]);

// Represents a base field element, using Fr as the backing type.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BaseElement(Fr);

impl FieldElement for BaseElement {
    type PositiveInteger = BigInt;
    type BaseField = Self;

    const ZERO: Self = BaseElement(Fr([0, 0, 0, 0]));
    const ONE: Self = BaseElement(Fr(R.0)); // equal to 2^256 mod M

    const ELEMENT_BYTES: usize = ELEMENT_BYTES;

    const IS_CANONICAL: bool = false;

    fn inv(self) -> Self {
        Self(self.0.invert().unwrap())
    }

    fn conjugate(&self) -> Self {
        Self(self.0)
    }

    fn elements_as_bytes(elements: &[Self]) -> &[u8] {
        // NOTE: elements are still in Montgomery form
        let p = elements.as_ptr();
        let len = elements.len() * Self::ELEMENT_BYTES;
        unsafe { slice::from_raw_parts(p as *const u8, len) }
    }

    unsafe fn bytes_as_elements(_bytes: &[u8]) -> Result<&[Self], DeserializationError> {
        unimplemented!()
    }

    fn zeroed_vector(n: usize) -> Vec<Self> {
        // TODO: use more efficient initialization
        let result = vec![Self::ZERO.0; n];

        // translate a zero-filled vector of Fr into a vector of base field elements
        let mut v = core::mem::ManuallyDrop::new(result);
        let p = v.as_mut_ptr();
        let len = v.len();
        let cap = v.capacity();
        unsafe { Vec::from_raw_parts(p as *mut Self, len, cap) }
    }

    fn as_base_elements(elements: &[Self]) -> &[Self::BaseField] {
        elements
    }
}

impl BaseElement {
    // Equal to 2*2^256 mod M (R is derived from the macro)
    pub const TWO: Self = BaseElement(Fr([
        0xffff_ffff_ffff_ffc1,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
        0x7fff_ffff_ffff_bd0,
    ]));

    pub fn from_raw(value: [u64; 4]) -> Self {
        BaseElement(Fr::from_raw(value))
    }

    pub fn to_raw(&self) -> BigInt {
        self.0.to_raw()
    }
}

impl StarkField for BaseElement {
    /// sage: MODULUS = 2^251 + 17 * 2^192 + 1 \
    /// sage: GF(MODULUS).is_prime_field() \
    /// True \
    /// sage: GF(MODULUS).order() \
    /// 3618502788666131213697322783095070105623107215331596699973092056135872020481
    const MODULUS: Self::PositiveInteger = BigInt([0x1, 0x0, 0x0, 0x8000_0000_0000_011]);
    const MODULUS_BITS: u32 = 252;

    /// sage: GF(MODULUS).primitive_element() \
    /// 3
    const GENERATOR: Self = BaseElement(GENERATOR);

    /// sage: is_odd((MODULUS - 1) / 2^192) \
    /// True
    const TWO_ADICITY: u32 = 192;

    /// sage: k = (MODULUS - 1) / 2^192 \
    /// sage: GF(MODULUS).primitive_element()^k \
    /// 145784604816374866144131285430889962727208297722245411306711449302875041684
    const TWO_ADIC_ROOT_OF_UNITY: Self = BaseElement(ROOT_OF_UNITY);

    fn get_modulus_le_bytes() -> Vec<u8> {
        Self::MODULUS.to_le_bytes()
    }

    /// Convert from Montgomery form
    #[inline]
    fn as_int(&self) -> Self::PositiveInteger {
        self.0.to_raw()
    }
}

impl Randomizable for BaseElement {
    const VALUE_SIZE: usize = Self::ELEMENT_BYTES;

    fn from_random_bytes(bytes: &[u8]) -> Option<Self> {
        Self::try_from(bytes).ok()
    }
}

impl Display for BaseElement {
    fn fmt(&self, f: &mut Formatter) -> core::fmt::Result {
        write!(f, "{}", self.to_raw())
    }
}

// OVERLOADED OPERATORS
// ================================================================================================

impl Add for BaseElement {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for BaseElement {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs
    }
}

impl Sub for BaseElement {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for BaseElement {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Mul for BaseElement {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0)
    }
}

impl MulAssign for BaseElement {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs
    }
}

impl Div for BaseElement {
    type Output = Self;

    fn div(self, rhs: Self) -> Self {
        Self(self.0 * rhs.0.invert().unwrap())
    }
}

impl DivAssign for BaseElement {
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs
    }
}

impl Neg for BaseElement {
    type Output = Self;

    fn neg(self) -> Self {
        Self(-self.0)
    }
}

// QUADRATIC EXTENSION
// ================================================================================================

/// Defines a quadratic extension of the base field over an irreducible polynomial x<sup>2</sup> -
/// x - 1. Thus, an extension element is defined as α + β * φ, where φ is a root of this polynomial,
/// and α and β are base field elements.
impl ExtensibleField<2> for BaseElement {
    #[inline(always)]
    fn mul(a: [Self; 2], b: [Self; 2]) -> [Self; 2] {
        let z = a[0] * b[0];
        [z + a[1] * b[1], (a[0] + a[1]) * (b[0] + b[1]) - z]
    }

    #[inline(always)]
    fn mul_base(a: [Self; 2], b: Self) -> [Self; 2] {
        [a[0] * b, a[1] * b]
    }

    #[inline(always)]
    fn frobenius(x: [Self; 2]) -> [Self; 2] {
        [x[0] + x[1], Self::ZERO - x[1]]
    }
}

// CUBIC EXTENSION
// ================================================================================================

/// Cubic extension for this field is not implemented as quadratic extension already provides
/// sufficient security level.
impl ExtensibleField<3> for BaseElement {
    fn mul(_a: [Self; 3], _b: [Self; 3]) -> [Self; 3] {
        unimplemented!()
    }

    #[inline(always)]
    fn mul_base(_a: [Self; 3], _b: Self) -> [Self; 3] {
        unimplemented!()
    }

    #[inline(always)]
    fn frobenius(_x: [Self; 3]) -> [Self; 3] {
        unimplemented!()
    }

    fn is_supported() -> bool {
        false
    }
}

// TYPE CONVERSIONS
// ================================================================================================

impl From<u128> for BaseElement {
    /// Converts a 128-bit value into a field element.
    fn from(value: u128) -> Self {
        let hi: u64 = (value >> 64) as u64;
        let lo: u64 = value as u64;
        Self(Fr::from_raw([lo, hi, 0, 0]))
    }
}

impl From<u64> for BaseElement {
    /// Converts a 64-bit value into a field element.
    fn from(value: u64) -> Self {
        Self(Fr::from_raw([value, 0, 0, 0]))
    }
}

impl From<u32> for BaseElement {
    /// Converts a 32-bit value into a field element.
    fn from(value: u32) -> Self {
        Self(Fr::from_raw([value as u64, 0, 0, 0]))
    }
}

impl From<u16> for BaseElement {
    /// Converts a 16-bit value into a field element.
    fn from(value: u16) -> Self {
        Self(Fr::from_raw([value as u64, 0, 0, 0]))
    }
}

impl From<u8> for BaseElement {
    /// Converts an 8-bit value into a field element.
    fn from(value: u8) -> Self {
        Self(Fr::from_raw([value as u64, 0, 0, 0]))
    }
}

impl From<usize> for BaseElement {
    /// Converts a usize value into a field element.
    fn from(value: usize) -> Self {
        Self(Fr::from_raw([value as u64, 0, 0, 0]))
    }
}

impl From<[u64; 4]> for BaseElement {
    /// Converts the value encoded in an array of 4 64-bit words into a field element. The bytes
    /// are assumed to be in little-endian byte order. If the value is greater than or equal
    /// to the field modulus, modular reduction is silently performed.
    fn from(bytes: [u64; 4]) -> Self {
        Self(Fr::from_raw(bytes))
    }
}

impl From<[u8; 32]> for BaseElement {
    /// Converts the value encoded in an array of 32 bytes into a field element. The bytes
    /// are assumed to be in little-endian byte order. If the value is greater than or equal
    /// to the field modulus, modular reduction is silently performed.
    fn from(bytes: [u8; 32]) -> Self {
        let value: [u64; 4] = bytes
            .array_chunks::<8>()
            .map(|c| u64::from_le_bytes(*c))
            .collect::<Vec<u64>>()
            .try_into()
            .unwrap();
        Self(Fr::from_raw(value))
    }
}

impl<'a> TryFrom<&'a [u8]> for BaseElement {
    type Error = String;

    /// Converts a slice of bytes into a field element. The bytes are assumed to be in
    /// little-endian byte order. If the value is greater than or equal to the field modulus,
    /// modular reduction is silently performed.
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let mut value: [u64; 4] = [0; 4];
        for (i, c) in bytes.chunks(8).enumerate() {
            value[i] = u64::from_le_bytes(TryInto::<[u8; 8]>::try_into(c).unwrap());
        }
        Ok(Self(Fr::from_raw(value)))
    }
}

// TODO: Why is this method not working? See commented lines in core/src/word/helpers.rs
impl AsBytes for BaseElement {
    fn as_bytes(&self) -> &[u8] {
        //let ptr: *const Vec<u8> = &self.as_int().to_le_bytes();
        //unsafe { slice::from_raw_parts(ptr as *const u8, ELEMENT_BYTES) }
        let ptr: *const BigInt = &self.to_raw();
        unsafe { slice::from_raw_parts(ptr as *const u8, ELEMENT_BYTES) }
    }
}

// SERIALIZATION / DESERIALIZATION
// ------------------------------------------------------------------------------------------------

impl Serializable for BaseElement {
    fn write_into<W: ByteWriter>(&self, target: &mut W) {
        target.write_u8_slice(&self.to_raw().to_le_bytes());
    }
}

impl Deserializable for BaseElement {
    fn read_from<R: ByteReader>(source: &mut R) -> Result<Self, DeserializationError> {
        let bytes: [u8; 32] = source.read_u8_array()?;
        let value: [u64; 4] = bytes
            .array_chunks::<8>()
            .map(|c| u64::from_le_bytes(*c))
            .collect::<Vec<u64>>()
            .try_into()
            .unwrap();
        Ok(BaseElement(Fr::from_raw(value)))
    }
}

// OVERLOADED OPERATORS (BIGINT)
// ================================================================================================

impl Shr<u32> for BigInt {
    type Output = BigInt;
    fn shr(self, rhs: u32) -> BigInt {
        shr_vartime(&self, rhs as usize)
    }
}

impl Shr<u32> for &BigInt {
    type Output = BigInt;
    fn shr(self, rhs: u32) -> BigInt {
        shr_vartime(&self, rhs as usize)
    }
}

impl ShrAssign for BigInt {
    fn shr_assign(&mut self, rhs: BigInt) {
        let shift: u64 = rhs.try_into().unwrap();
        *self = shr_vartime(&self, shift as usize);
    }
}

impl Shl<u32> for BigInt {
    type Output = BigInt;
    fn shl(self, rhs: u32) -> BigInt {
        shl_vartime(&self, rhs as usize)
    }
}

impl Shl<u32> for &BigInt {
    type Output = BigInt;
    fn shl(self, rhs: u32) -> BigInt {
        shl_vartime(&self, rhs as usize)
    }
}

impl BitAnd for BigInt {
    type Output = Self;
    fn bitand(self, Self(rhs): Self) -> Self::Output {
        let mut limbs = [0u64; 4];
        for i in 0..4 {
            limbs[i] = self.0[i] & rhs[i];
        }
        Self(limbs)
    }
}

// Modified from https://github.com/RustCrypto/crypto-bigint/blob/master/src/uint/shr.rs
fn shr_vartime(value: &BigInt, shift: usize) -> BigInt {
    let full_shifts = shift / 64;
    let small_shift = shift & (64 - 1);
    let mut limbs = [0u64; 4];

    if shift > 64 * 4 {
        return BigInt(limbs);
    }

    let n = 4 - full_shifts;
    let mut i = 0;

    if small_shift == 0 {
        while i < n {
            limbs[i] = value.0[i + full_shifts];
            i += 1;
        }
    } else {
        while i < n {
            let mut lo = value.0[i + full_shifts] >> small_shift;

            if i < (4 - 1) - full_shifts {
                lo |= value.0[i + full_shifts + 1] << (64 - small_shift);
            }

            limbs[i] = lo;
            i += 1;
        }
    }
    BigInt(limbs)
}

// Modified from https://github.com/RustCrypto/crypto-bigint/blob/171f6745b98b6dccf05f7d25263981949967f398/src/uint/shl.rs
fn shl_vartime(value: &BigInt, n: usize) -> BigInt {
    let mut limbs = [0u64; 4];

    if n >= 64 * 4 {
        return BigInt(limbs);
    }

    let shift_num = n / 64;
    let lshift_rem = n % 64;
    let nz = lshift_rem == 0;
    let rshift_rem = if nz { 0 } else { 64 - lshift_rem };
    let mut i = 4 - 1;
    while i > shift_num {
        let mut limb = value.0[i - shift_num] << lshift_rem;
        let hi = value.0[i - shift_num - 1] >> rshift_rem;
        limb |= hi & nz as u64;
        limbs[i] = limb;
        i -= 1
    }
    limbs[shift_num] = value.0[0] << lshift_rem;
    BigInt(limbs)
}

// TYPE CONVERSIONS (BIGINT, FR)
// ------------------------------------------------------------------------------------------------

impl From<u128> for BigInt {
    /// Converts a 128-bit value into a field element.
    fn from(value: u128) -> Self {
        let hi: u64 = (value >> 64) as u64;
        let lo: u64 = value as u64;
        BigInt([lo, hi, 0, 0])
    }
}

impl From<u64> for BigInt {
    /// Converts a 64-bit value into a field element.
    fn from(value: u64) -> Self {
        BigInt([value, 0, 0, 0])
    }
}

impl From<u32> for BigInt {
    /// Converts a 32-bit value into a field element.
    fn from(value: u32) -> Self {
        BigInt([value as u64, 0, 0, 0])
    }
}

impl From<u16> for BigInt {
    /// Converts a 16-bit value into a field element.
    fn from(value: u16) -> Self {
        BigInt([value as u64, 0, 0, 0])
    }
}

impl From<u8> for BigInt {
    /// Converts an 8-bit value into a field element.
    fn from(value: u8) -> Self {
        BigInt([value as u64, 0, 0, 0])
    }
}

impl TryInto<u64> for BigInt {
    type Error = ();
    fn try_into(self) -> Result<u64, Self::Error> {
        Ok(self.0[0])
    }
}

impl TryInto<u16> for BigInt {
    type Error = ();
    fn try_into(self) -> Result<u16, Self::Error> {
        Ok(self.0[0] as u16)
    }
}

impl BigInt {
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut result = [0u8; 32];
        write_le_bytes(self.0, &mut result);
        result.to_vec()
    }
}

impl Display for BigInt {
    fn fmt(&self, f: &mut Formatter) -> core::fmt::Result {
        for i in (0..4).rev() {
            write!(f, "{:016x}", &self.0[i]);
        }
        Ok(())
    }
}

impl Fr {
    pub fn from_raw(value: [u64; 4]) -> Self {
        Fr(value).mul(&R2)
    }

    pub fn to_raw(&self) -> BigInt {
        let limbs = self.0;
        let mut val = self.clone();
        val.mont_reduce(limbs[0], limbs[1], limbs[2], limbs[3], 0, 0, 0, 0);
        BigInt(val.0)
    }

    pub fn to_le_bytes(&self) -> Vec<u8> {
        let mut result = [0u8; 32];
        write_le_bytes(self.0, &mut result);
        result.to_vec()
    }
}

// Modified from https://github.com/RustCrypto/crypto-bigint/blob/171f6745b98b6dccf05f7d25263981949967f398/src/uint/encoding.rs
fn write_le_bytes(value: [u64; 4], out: &mut [u8]) {
    for (src, dst) in value.iter().cloned().zip(out.chunks_exact_mut(8)) {
        dst.copy_from_slice(&src.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_int() {
        let a = BaseElement::from_raw([3, 0, 0, 0]);
        let b: u64 = a.as_int().try_into().unwrap();
        assert_eq!(b, 3);
    }
}
