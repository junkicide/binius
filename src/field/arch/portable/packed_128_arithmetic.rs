// Copyright 2024 Ulvetanna Inc.

use crate::field::{
	arch::portable::packed_128::PackedBinaryField128x1b, BinaryField, ExtensionField, PackedField,
	TowerField,
};

/// Abstraction for a packed tower field of height more than 1 that is represented by a 128-bit integer.
/// (Actually this trait itslef may be generalized for any underlier)
pub trait PackedTowerField: PackedField + From<u128> + Into<u128> {
	/// A scalar of a lower height
	type DirectSubfield: TowerField;
	/// Packed type with the same underlier with a lower height
	type PackedDirectSubfield: PackedField<Scalar = Self::DirectSubfield>
		+ PackedMultiply
		+ From<u128>
		+ Into<u128>;

	/// Reinterpret value as a packed field over a lower height
	fn as_packed_subfield(self) -> Self::PackedDirectSubfield {
		Self::PackedDirectSubfield::from(Into::<u128>::into(self))
	}
}

/// A trait for packed field that implements packed multiplication
pub(super) trait PackedMultiply {
	fn packed_multiply(a: Self, b: Self) -> Self;
}

impl PackedMultiply for PackedBinaryField128x1b {
	fn packed_multiply(a: Self, b: Self) -> Self {
		(u128::from(a) & u128::from(b)).into()
	}
}

/// Compile-time known constants needed for packed multiply implementation.
trait PackedMultiplyConstants {
	const ALPHAS: u128;
	const INTERLEAVE_MASK: u128;
}

impl<F: TowerField> PackedMultiplyConstants for F {
	const ALPHAS: u128 = generate_alphas_even::<F>();
	const INTERLEAVE_MASK: u128 = generate_interleave_mask::<F>();
}

impl<PT> PackedMultiply for PT
where
	PT: PackedTowerField,
{
	/// Optimized packed field multiplication algorithm
	fn packed_multiply(a: Self, b: Self) -> Self {
		assert_ne!(PT::DirectSubfield::DEGREE, 0);

		// a and b can be interpreted as packed subfield elements:
		// a = <a_lo_0, a_hi_0, a_lo_1, a_hi_1, ...>
		// b = <b_lo_0, b_hi_0, b_lo_1, b_hi_1, ...>//
		// ab is the product of a * b as packed subfield elements
		// ab = <a_lo_0 * b_lo_0, a_hi_0 * b_hi_0, a_lo_1 * b_lo_1, a_hi_1 * b_hi_1, ...>
		let repacked_a = a.as_packed_subfield();
		let repacked_b = b.as_packed_subfield();
		let z0_even_z2_odd = repacked_a * repacked_b;

		// lo = <a_lo_0, b_lo_0, a_lo_1, b_lo_1, ...>
		// hi = <a_hi_0, b_hi_0, a_hi_1, b_hi_1, ...>
		let (lo, hi) = interleave::<PT::DirectSubfield>(a.into(), b.into());

		// <a_lo_0 + a_hi_0, b_lo_0 + b_hi_0, a_lo_1 + a_hi_1, b_lo_1 + b_hi_1, ...>
		let lo_plus_hi_a_even_b_odd = lo ^ hi;

		let even_mask = PT::DirectSubfield::INTERLEAVE_MASK;
		let odd_mask = even_mask << PT::DirectSubfield::N_BITS;

		let alphas = PT::DirectSubfield::ALPHAS;

		// <α, z2_0, α, z2_1, ...>
		let alpha_even_z2_odd = alphas ^ (z0_even_z2_odd.into() & odd_mask);

		// a_lo_plus_hi_even_z2_odd    = <a_lo_0 + a_hi_0, z2_0, a_lo_1 + a_hi_1, z2_1, ...>
		// b_lo_plus_hi_even_alpha_odd = <b_lo_0 + b_hi_0,    α, a_lo_1 + a_hi_1,   αz, ...>
		let (a_lo_plus_hi_even_alpha_odd, b_lo_plus_hi_even_z2_odd) =
			interleave::<PT::DirectSubfield>(lo_plus_hi_a_even_b_odd, alpha_even_z2_odd);

		// <z1_0 + z0_0 + z2_0, z2a_0, z1_1 + z0_1 + z2_1, z2a_1, ...>
		let z1_plus_z0_plus_z2_even_z2a_odd =
			PT::PackedDirectSubfield::from(a_lo_plus_hi_even_alpha_odd)
				* PT::PackedDirectSubfield::from(b_lo_plus_hi_even_z2_odd);

		// <0, z1_0 + z2a_0 + z0_0 + z2_0, 0, z1_1 + z2a_1 + z0_1 + z2_1, ...>
		let zero_even_z1_plus_z2a_plus_z0_plus_z2_odd = (z1_plus_z0_plus_z2_even_z2a_odd.into()
			^ (z1_plus_z0_plus_z2_even_z2a_odd.into() << PT::DirectSubfield::N_BITS))
			& odd_mask;

		// <z0_0 + z2_0, z0_0 + z2_0, z0_1 + z2_1, z0_1 + z2_1, ...>
		let z0_plus_z2_dup = xor_adjacent::<PT::DirectSubfield>(z0_even_z2_odd.into());

		// <z0_0 + z2_0, z1_0 + z2a_0, z0_1 + z2_1, z1_1 + z2a_1, ...>
		(z0_plus_z2_dup ^ zero_even_z1_plus_z2a_plus_z0_plus_z2_odd).into()
	}
}

/// Generate the mask with ones in the odd packed element positions and zeros in even
const fn generate_interleave_mask<F: BinaryField>() -> u128 {
	let mut mask = (1u128 << F::N_BITS) - 1u128;
	let log_width = (u128::BITS as usize / F::N_BITS).ilog2() as usize;
	let mut i = 1;
	while i < log_width {
		mask |= mask << (F::N_BITS << i);
		i += 1;
	}
	mask
}

/// View the inputs as vectors of packed binary tower elements and transpose as 2x2 square matrices.
/// Given vectors <a_0, a_1, a_2, a_3, ...> and <b_0, b_1, b_2, b_3, ...>, returns a tuple with
/// <a0, b0, a2, b2, ...> and <a1, b1, a3, b3>.
fn interleave<F: TowerField>(a: u128, b: u128) -> (u128, u128) {
	let mask = F::INTERLEAVE_MASK;

	let block_len = 1 << F::TOWER_LEVEL;
	let t = ((a >> block_len) ^ b) & mask;
	let c = a ^ (t << block_len);
	let d = b ^ t;
	(c, d)
}

/// Generate the packed value with alpha in the even positions and zero in the odd positions.
const fn generate_alphas_even<F: TowerField>() -> u128 {
	let mut alphas = if F::TOWER_LEVEL == 0 {
		1u128
	} else {
		1u128 << (1 << (F::TOWER_LEVEL - 1))
	};

	let log_width = (u128::BITS as usize / F::N_BITS).ilog2() as usize;
	let mut i = 1;
	while i < log_width {
		alphas |= alphas << (1 << (F::TOWER_LEVEL + i));
		i += 1;
	}

	alphas
}

/// View the input as a vector of packed binary tower elements and add the adjacent ones.
/// Given a vector <a_0, a_1, a_2, a_3, ...>, returns <a0 + a1, a0 + a1, a2 + a3, a2 + a3, ...>.
fn xor_adjacent<F: TowerField>(a: u128) -> u128 {
	let mask = F::INTERLEAVE_MASK;

	let block_len = F::N_BITS;
	let t = ((a >> block_len) ^ a) & mask;

	t ^ (t << block_len)
}

#[cfg(test)]
mod tests {
	use super::*;

	use rand::thread_rng;
	use std::fmt::Debug;

	use crate::field::arch::portable::packed_128::{
		PackedBinaryField16x8b, PackedBinaryField1x128b, PackedBinaryField2x64b,
		PackedBinaryField32x4b, PackedBinaryField4x32b, PackedBinaryField64x2b,
		PackedBinaryField8x16b,
	};

	use crate::field::{
		BinaryField16b, BinaryField1b, BinaryField2b, BinaryField32b, BinaryField4b,
		BinaryField64b, BinaryField8b,
	};

	#[test]
	fn test_generate_interleave_mask() {
		assert_eq!(generate_interleave_mask::<BinaryField1b>(), 0x55555555555555555555555555555555);
		assert_eq!(generate_interleave_mask::<BinaryField8b>(), 0x00FF00FF00FF00FF00FF00FF00FF00FF);
		assert_eq!(
			generate_interleave_mask::<BinaryField64b>(),
			0x0000000000000000FFFFFFFFFFFFFFFF
		);
	}

	fn test_packed_multiply<P>()
	where
		P: PackedField + PackedMultiply + Debug,
	{
		let mut rng = thread_rng();
		let a = P::random(&mut rng);
		let b = P::random(&mut rng);

		let result = P::packed_multiply(a, b);
		for i in 0..P::WIDTH {
			assert_eq!(result.get(i), a.get(i) * b.get(i));
		}
	}

	#[test]
	fn test_multiply() {
		test_packed_multiply::<PackedBinaryField128x1b>();
		test_packed_multiply::<PackedBinaryField64x2b>();
		test_packed_multiply::<PackedBinaryField32x4b>();
		test_packed_multiply::<PackedBinaryField16x8b>();
		test_packed_multiply::<PackedBinaryField8x16b>();
		test_packed_multiply::<PackedBinaryField4x32b>();
		test_packed_multiply::<PackedBinaryField2x64b>();
		test_packed_multiply::<PackedBinaryField1x128b>();
	}

	fn check_interleave<F: TowerField>(a: u128, b: u128, c: u128, d: u128) {
		assert_eq!(interleave::<F>(a, b), (c, d));
		assert_eq!(interleave::<F>(c, d), (a, b));
	}

	#[test]
	fn test_interleave() {
		check_interleave::<BinaryField1b>(
			0x0000000000000000FFFFFFFFFFFFFFFF,
			0xFFFFFFFFFFFFFFFF0000000000000000,
			0xAAAAAAAAAAAAAAAA5555555555555555,
			0xAAAAAAAAAAAAAAAA5555555555555555,
		);

		check_interleave::<BinaryField2b>(
			0x0000000000000000FFFFFFFFFFFFFFFF,
			0xFFFFFFFFFFFFFFFF0000000000000000,
			0xCCCCCCCCCCCCCCCC3333333333333333,
			0xCCCCCCCCCCCCCCCC3333333333333333,
		);

		check_interleave::<BinaryField4b>(
			0x0000000000000000FFFFFFFFFFFFFFFF,
			0xFFFFFFFFFFFFFFFF0000000000000000,
			0xF0F0F0F0F0F0F0F00F0F0F0F0F0F0F0F,
			0xF0F0F0F0F0F0F0F00F0F0F0F0F0F0F0F,
		);

		check_interleave::<BinaryField8b>(
			0x0F0E0D0C0B0A09080706050403020100,
			0x1F1E1D1C1B1A19181716151413121110,
			0x1E0E1C0C1A0A18081606140412021000,
			0x1F0F1D0D1B0B19091707150513031101,
		);

		check_interleave::<BinaryField16b>(
			0x0F0E0D0C0B0A09080706050403020100,
			0x1F1E1D1C1B1A19181716151413121110,
			0x1D1C0D0C191809081514050411100100,
			0x1F1E0F0E1B1A0B0A1716070613120302,
		);

		check_interleave::<BinaryField32b>(
			0x0F0E0D0C0B0A09080706050403020100,
			0x1F1E1D1C1B1A19181716151413121110,
			0x1B1A19180B0A09081312111003020100,
			0x1F1E1D1C0F0E0D0C1716151407060504,
		);

		check_interleave::<BinaryField64b>(
			0x0F0E0D0C0B0A09080706050403020100,
			0x1F1E1D1C1B1A19181716151413121110,
			0x17161514131211100706050403020100,
			0x1F1E1D1C1B1A19180F0E0D0C0B0A0908,
		);
	}
}
