use std::collections::{HashMap, HashSet};

use crate::config::SearchConfig;

use super::math::{
    deterministic_hyperplane_seed, generate_hyperplanes, signature_f32, signature_i8,
    validate_hyperplanes,
};

pub(super) enum DenseEmbeddings {
    Float(Vec<(String, Vec<f32>)>),
    Quantized(Vec<QuantizedEmbedding>),
}

pub(super) struct QuantizedEmbedding {
    pub(super) id: String,
    pub(super) values: Vec<i8>,
}

pub(super) struct AnnIndex {
    pub(super) hyperplanes: Vec<Vec<f32>>,
    pub(super) buckets: HashMap<u64, Vec<usize>>,
    bits: usize,
    max_candidates: usize,
    min_candidates: usize,
    hamming: usize,
}

impl AnnIndex {
    pub(super) fn build_f32(
        embeddings: &[(String, Vec<f32>)],
        config: &SearchConfig,
    ) -> Option<Self> {
        if !config.enable_vector_ann || embeddings.is_empty() {
            return None;
        }

        let dim = embeddings.first().map_or(0, |(_, v)| v.len());
        if dim == 0 {
            return None;
        }

        let bits = config.vector_ann_bits.clamp(4, 63);
        let seed = deterministic_hyperplane_seed(bits, dim);
        let hyperplanes = generate_hyperplanes(bits, dim, seed);
        Self::build_f32_with_hyperplanes(embeddings, config, hyperplanes)
    }

    pub(super) fn build_f32_with_hyperplanes(
        embeddings: &[(String, Vec<f32>)],
        config: &SearchConfig,
        hyperplanes: Vec<Vec<f32>>,
    ) -> Option<Self> {
        if !config.enable_vector_ann || embeddings.is_empty() {
            return None;
        }
        let bits = config.vector_ann_bits.clamp(4, 63);
        let dim = embeddings.first().map_or(0, |(_, v)| v.len());
        let hyperplanes = validate_hyperplanes(hyperplanes, bits, dim)?;
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();

        for (idx, (_, vec)) in embeddings.iter().enumerate() {
            let sig = signature_f32(vec, &hyperplanes);
            buckets.entry(sig).or_default().push(idx);
        }

        Some(Self {
            hyperplanes,
            buckets,
            bits,
            max_candidates: config.vector_ann_max_candidates.max(1),
            min_candidates: config.vector_ann_min_candidates,
            hamming: config.vector_ann_hamming.min(bits),
        })
    }

    pub(super) fn build_quantized(
        embeddings: &[QuantizedEmbedding],
        config: &SearchConfig,
    ) -> Option<Self> {
        if !config.enable_vector_ann || embeddings.is_empty() {
            return None;
        }

        let dim = embeddings.first().map_or(0, |e| e.values.len());
        if dim == 0 {
            return None;
        }

        let bits = config.vector_ann_bits.clamp(4, 63);
        let seed = deterministic_hyperplane_seed(bits, dim);
        let hyperplanes = generate_hyperplanes(bits, dim, seed);
        Self::build_quantized_with_hyperplanes(embeddings, config, hyperplanes)
    }

    pub(super) fn build_quantized_with_hyperplanes(
        embeddings: &[QuantizedEmbedding],
        config: &SearchConfig,
        hyperplanes: Vec<Vec<f32>>,
    ) -> Option<Self> {
        if !config.enable_vector_ann || embeddings.is_empty() {
            return None;
        }
        let bits = config.vector_ann_bits.clamp(4, 63);
        let dim = embeddings.first().map_or(0, |e| e.values.len());
        let hyperplanes = validate_hyperplanes(hyperplanes, bits, dim)?;
        let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();

        for (idx, embedding) in embeddings.iter().enumerate() {
            let sig = signature_i8(&embedding.values, &hyperplanes);
            buckets.entry(sig).or_default().push(idx);
        }

        Some(Self {
            hyperplanes,
            buckets,
            bits,
            max_candidates: config.vector_ann_max_candidates.max(1),
            min_candidates: config.vector_ann_min_candidates,
            hamming: config.vector_ann_hamming.min(bits),
        })
    }

    pub(super) fn cache_bucket_parts(&self) -> (Vec<u64>, Vec<Vec<usize>>) {
        let mut entries = self
            .buckets
            .iter()
            .map(|(key, values)| (*key, values.clone()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(key, _)| *key);
        entries.into_iter().unzip()
    }

    pub(super) fn candidates_f32(&self, query_vec: &[f32], top_k: usize) -> Option<Vec<usize>> {
        let sig = signature_f32(query_vec, &self.hyperplanes);
        self.candidates_for_signature(sig, top_k)
    }

    pub(super) fn candidates_i8(&self, query_vec: &[i8], top_k: usize) -> Option<Vec<usize>> {
        let sig = signature_i8(query_vec, &self.hyperplanes);
        self.candidates_for_signature(sig, top_k)
    }

    fn candidates_for_signature(&self, sig: u64, top_k: usize) -> Option<Vec<usize>> {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut candidates: Vec<usize> = Vec::new();

        if self.push_bucket(sig, &mut candidates, &mut seen) {
            return Some(candidates);
        }

        if self.hamming >= 1 {
            for bit in 0..self.bits {
                if self.push_bucket(sig ^ (1u64 << bit), &mut candidates, &mut seen) {
                    return Some(candidates);
                }
            }
        }

        if self.hamming >= 2 {
            for i in 0..self.bits {
                for j in (i + 1)..self.bits {
                    if self.push_bucket(sig ^ (1u64 << i) ^ (1u64 << j), &mut candidates, &mut seen)
                    {
                        return Some(candidates);
                    }
                }
            }
        }

        let min_needed = self.min_candidates.max(top_k.saturating_mul(4));
        if candidates.len() < min_needed {
            return None;
        }

        Some(candidates)
    }

    fn push_bucket(
        &self,
        bucket: u64,
        candidates: &mut Vec<usize>,
        seen: &mut HashSet<usize>,
    ) -> bool {
        if let Some(ids) = self.buckets.get(&bucket) {
            for id in ids {
                if seen.insert(*id) {
                    candidates.push(*id);
                    if candidates.len() >= self.max_candidates {
                        return true;
                    }
                }
            }
        }
        false
    }
}
