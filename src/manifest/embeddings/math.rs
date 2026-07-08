use fastembed::SparseEmbedding;

pub(super) fn normalize_vector(vec: &mut [f32]) {
    let norm = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

pub(super) fn deterministic_hyperplane_seed(bits: usize, dim: usize) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"dbt-nova/vector-ann/v1");
    hasher.update(&bits.to_le_bytes());
    hasher.update(&dim.to_le_bytes());
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash.as_bytes()[..8]);
    u64::from_le_bytes(bytes)
}

pub(super) fn validate_hyperplanes(
    hyperplanes: Vec<Vec<f32>>,
    bits: usize,
    dim: usize,
) -> Option<Vec<Vec<f32>>> {
    if hyperplanes.len() != bits {
        return None;
    }
    if hyperplanes.iter().any(|plane| plane.len() != dim) {
        return None;
    }
    Some(hyperplanes)
}

pub(super) fn generate_hyperplanes(bits: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = XorShift64::new(seed);
    let mut planes = Vec::with_capacity(bits);
    for _ in 0..bits {
        let mut vec = Vec::with_capacity(dim);
        for _ in 0..dim {
            vec.push(rng.next_f32());
        }
        normalize_vector(&mut vec);
        planes.push(vec);
    }
    planes
}

pub(super) struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        let seed = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn next_f32(&mut self) -> f32 {
        let value = self.next_u64();
        let unit = (value as f64) / (u64::MAX as f64);
        (unit * 2.0 - 1.0) as f32
    }
}

pub(super) const QUANT_SCALE: f32 = 1.0 / 127.0;

#[allow(clippy::cast_possible_truncation)]
pub(super) fn quantize_vector(vec: &[f32]) -> Vec<i8> {
    vec.iter()
        .map(|v| (v * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

pub(super) fn top_k_scored(mut scored: Vec<(usize, f32)>, top_k: usize) -> Vec<(usize, f32)> {
    if top_k == 0 || scored.is_empty() {
        return Vec::new();
    }
    if scored.len() > top_k {
        scored.select_nth_unstable_by(top_k, score_desc_cmp);
        scored.truncate(top_k);
    }
    scored.sort_by(score_desc_cmp);
    scored
}

pub(super) fn score_desc_cmp(a: &(usize, f32), b: &(usize, f32)) -> std::cmp::Ordering {
    let score_a = if a.1.is_finite() {
        a.1
    } else {
        f32::NEG_INFINITY
    };
    let score_b = if b.1.is_finite() {
        b.1
    } else {
        f32::NEG_INFINITY
    };
    match score_b.total_cmp(&score_a) {
        std::cmp::Ordering::Equal => a.0.cmp(&b.0),
        other => other,
    }
}

#[inline]
pub(super) fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: `a` and `b` are valid slices; we only read up to `min(len)` and use
            // unaligned loads (`*_loadu_ps`), so alignment is not required.
            unsafe { return dot_product_avx2(a, b) };
        }
        if std::is_x86_feature_detected!("sse") {
            // SAFETY: `a` and `b` are valid slices; we only read up to `min(len)` and use
            // unaligned loads (`*_loadu_ps`), so alignment is not required.
            unsafe { return dot_product_sse(a, b) };
        }
    }
    dot_product_scalar(a, b)
}

#[inline]
pub(super) fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn dot_product_i8(a: &[i8], b: &[i8]) -> f32 {
    let mut acc: i32 = 0;
    let len = a.len().min(b.len());
    for i in 0..len {
        acc += i32::from(a[i]) * i32::from(b[i]);
    }
    (acc as f32) * (QUANT_SCALE * QUANT_SCALE)
}

#[cfg(target_arch = "x86_64")]
#[inline]
pub(super) unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: `a` and `b` are valid for at least `len` elements. The loop bounds ensure
    // `_mm256_loadu_ps` reads within the slice, and the remainder loop handles any tail.
    use std::arch::x86_64::{
        _mm256_add_ps, _mm256_loadu_ps, _mm256_mul_ps, _mm256_setzero_ps, _mm256_storeu_ps,
    };
    let len = a.len().min(b.len());
    // SAFETY: caller guarantees AVX2 support before calling this function.
    let mut sum = unsafe { _mm256_setzero_ps() };
    let mut i = 0usize;
    while i + 8 <= len {
        // SAFETY: `i + 8 <= len` guarantees each 8-f32 unaligned load is in-bounds.
        let va = unsafe { _mm256_loadu_ps(a[i..].as_ptr()) };
        // SAFETY: `i + 8 <= len` guarantees each 8-f32 unaligned load is in-bounds.
        let vb = unsafe { _mm256_loadu_ps(b[i..].as_ptr()) };
        // SAFETY: AVX2 support is guaranteed by caller.
        let prod = unsafe { _mm256_mul_ps(va, vb) };
        // SAFETY: AVX2 support is guaranteed by caller.
        sum = unsafe { _mm256_add_ps(sum, prod) };
        i += 8;
    }
    let mut tmp = [0f32; 8];
    // SAFETY: `tmp` has capacity for 8 f32 values.
    unsafe { _mm256_storeu_ps(tmp.as_mut_ptr(), sum) };
    let mut total = tmp.iter().sum::<f32>();
    for j in i..len {
        total += a[j] * b[j];
    }
    total
}

#[cfg(target_arch = "x86_64")]
#[inline]
pub(super) unsafe fn dot_product_sse(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: `a` and `b` are valid for at least `len` elements. The loop bounds ensure
    // `_mm_loadu_ps` reads within the slice, and the remainder loop handles any tail.
    use std::arch::x86_64::{_mm_add_ps, _mm_loadu_ps, _mm_mul_ps, _mm_setzero_ps, _mm_storeu_ps};
    let len = a.len().min(b.len());
    // SAFETY: caller guarantees SSE support before calling this function.
    let mut sum = unsafe { _mm_setzero_ps() };
    let mut i = 0usize;
    while i + 4 <= len {
        // SAFETY: `i + 4 <= len` guarantees each 4-f32 unaligned load is in-bounds.
        let va = unsafe { _mm_loadu_ps(a[i..].as_ptr()) };
        // SAFETY: `i + 4 <= len` guarantees each 4-f32 unaligned load is in-bounds.
        let vb = unsafe { _mm_loadu_ps(b[i..].as_ptr()) };
        // SAFETY: SSE support is guaranteed by caller.
        let prod = unsafe { _mm_mul_ps(va, vb) };
        // SAFETY: SSE support is guaranteed by caller.
        sum = unsafe { _mm_add_ps(sum, prod) };
        i += 4;
    }
    let mut tmp = [0f32; 4];
    // SAFETY: `tmp` has capacity for 4 f32 values.
    unsafe { _mm_storeu_ps(tmp.as_mut_ptr(), sum) };
    let mut total = tmp.iter().sum::<f32>();
    for j in i..len {
        total += a[j] * b[j];
    }
    total
}

pub(super) fn signature_f32(vec: &[f32], hyperplanes: &[Vec<f32>]) -> u64 {
    let mut sig = 0u64;
    for (i, plane) in hyperplanes.iter().enumerate() {
        let dot = dot_product(vec, plane);
        if dot >= 0.0 {
            sig |= 1u64 << i;
        }
    }
    sig
}

pub(super) fn signature_i8(vec: &[i8], hyperplanes: &[Vec<f32>]) -> u64 {
    let mut sig = 0u64;
    for (i, plane) in hyperplanes.iter().enumerate() {
        let mut dot = 0.0f32;
        let len = vec.len().min(plane.len());
        for j in 0..len {
            dot += f32::from(vec[j]) * QUANT_SCALE * plane[j];
        }
        if dot >= 0.0 {
            sig |= 1u64 << i;
        }
    }
    sig
}

pub(super) fn sparse_dot(query: &SparseEmbedding, doc: &SparseEmbedding) -> f32 {
    let mut score = 0.0f32;
    let mut qi = 0usize;
    let mut di = 0usize;

    while qi < query.indices.len() && di < doc.indices.len() {
        let q_idx = query.indices[qi];
        let d_idx = doc.indices[di];
        match q_idx.cmp(&d_idx) {
            std::cmp::Ordering::Equal => {
                score += query.values[qi] * doc.values[di];
                qi += 1;
                di += 1;
            }
            std::cmp::Ordering::Less => {
                qi += 1;
            }
            std::cmp::Ordering::Greater => {
                di += 1;
            }
        }
    }

    score
}
