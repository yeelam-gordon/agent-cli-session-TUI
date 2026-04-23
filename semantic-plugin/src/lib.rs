//! Semantic search plugin for agent-session-tui.
//!
//! Built as a cdylib (DLL/SO/dylib) that the main TUI loads at runtime.
//! Uses fastembed with all-MiniLM-L6-v2 for lightweight sentence embeddings.

use std::ffi::{CStr, c_char, c_float, c_int};
use std::sync::Mutex;

use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};

static MODEL: Mutex<Option<TextEmbedding>> = Mutex::new(None);
static DIM: Mutex<i32> = Mutex::new(0);

/// Initialize the embedding model.
/// `cache_dir` is the directory to download/cache the model files.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn semantic_init(cache_dir: *const c_char) -> c_int {
    let cache = if cache_dir.is_null() {
        None
    } else {
        let s = unsafe { CStr::from_ptr(cache_dir) };
        s.to_str().ok().map(|s| std::path::PathBuf::from(s))
    };

    let mut opts = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        .with_show_download_progress(true);
    if let Some(ref dir) = cache {
        opts = opts.with_cache_dir(dir.clone());
    }

    match TextEmbedding::try_new(opts) {
        Ok(mut model) => {
            // Get embedding dimension from a test embed
            match model.embed(vec!["test"], None) {
                Ok(vecs) if !vecs.is_empty() => {
                    let dim = vecs[0].len() as i32;
                    *DIM.lock().unwrap() = dim;
                    *MODEL.lock().unwrap() = Some(model);
                    0
                }
                _ => -1,
            }
        }
        Err(_) => -1,
    }
}

/// Return the embedding dimension (0 if not initialized).
#[unsafe(no_mangle)]
pub extern "C" fn semantic_dim() -> c_int {
    *DIM.lock().unwrap()
}

/// Unload the embedding model, freeing all weights and ONNX runtime state.
/// Safe to call even if never initialized. After unload, `semantic_init`
/// can be called again to reload (useful for reducing idle memory).
/// Returns 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn semantic_unload() -> c_int {
    // Dropping the model releases its ~90MB of weights + ONNX session
    // back to the OS allocator.
    *MODEL.lock().unwrap() = None;
    0
}

/// Embed a single text string.
/// `text` is a null-terminated UTF-8 C string.
/// `out_vec` is a pre-allocated float buffer of at least `max_dim` elements.
/// Returns the actual dimension written, or -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn semantic_embed(
    text: *const c_char,
    out_vec: *mut c_float,
    max_dim: c_int,
) -> c_int {
    if text.is_null() || out_vec.is_null() {
        return -1;
    }

    let s = match unsafe { CStr::from_ptr(text) }.to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -1,
    };

    let mut guard = MODEL.lock().unwrap();
    let model = match guard.as_mut() {
        Some(m) => m,
        None => return -1,
    };

    match model.embed(vec![s], None) {
        Ok(vecs) if !vecs.is_empty() => {
            let dim = vecs[0].len().min(max_dim as usize);
            let out_slice = unsafe { std::slice::from_raw_parts_mut(out_vec, dim) };
            out_slice.copy_from_slice(&vecs[0][..dim]);
            dim as c_int
        }
        _ => -1,
    }
}

/// Compute cosine similarity between two vectors.
/// Returns the similarity score (-1.0 to 1.0), or -2.0 on error.
#[unsafe(no_mangle)]
pub extern "C" fn semantic_cosine(
    vec_a: *const c_float,
    vec_b: *const c_float,
    dim: c_int,
) -> c_float {
    if vec_a.is_null() || vec_b.is_null() || dim <= 0 {
        return -2.0;
    }

    let a = unsafe { std::slice::from_raw_parts(vec_a, dim as usize) };
    let b = unsafe { std::slice::from_raw_parts(vec_b, dim as usize) };

    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a < 1e-10 || norm_b < 1e-10 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Embed multiple texts at once (batch).
/// `texts_json` is a null-terminated JSON array of strings: ["text1", "text2", ...]
/// `out_vecs` is a pre-allocated buffer of (count * dim) floats.
/// `max_count` is the max number of texts to embed.
/// Returns the number of texts embedded, or -1 on error.
#[unsafe(no_mangle)]
pub extern "C" fn semantic_embed_batch(
    texts_json: *const c_char,
    out_vecs: *mut c_float,
    max_count: c_int,
) -> c_int {
    if texts_json.is_null() || out_vecs.is_null() {
        return -1;
    }

    let json_str = match unsafe { CStr::from_ptr(texts_json) }.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    let texts: Vec<String> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return -1,
    };

    let count = texts.len().min(max_count as usize);
    let texts_slice: Vec<String> = texts.into_iter().take(count).collect();

    let mut guard = MODEL.lock().unwrap();
    let model = match guard.as_mut() {
        Some(m) => m,
        None => return -1,
    };

    let dim = *DIM.lock().unwrap() as usize;
    if dim == 0 {
        return -1;
    }

    match model.embed(texts_slice, None) {
        Ok(vecs) => {
            let out_slice = unsafe {
                std::slice::from_raw_parts_mut(out_vecs, count * dim)
            };
            for (i, vec) in vecs.iter().enumerate().take(count) {
                let start = i * dim;
                let end = start + dim.min(vec.len());
                out_slice[start..end].copy_from_slice(&vec[..dim.min(vec.len())]);
            }
            vecs.len().min(count) as c_int
        }
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let a = [1.0f32, 0.0, 0.0];
        let b = [1.0f32, 0.0, 0.0];
        let sim = semantic_cosine(a.as_ptr(), b.as_ptr(), 3);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = [1.0f32, 0.0, 0.0];
        let b = [0.0f32, 1.0, 0.0];
        let sim = semantic_cosine(a.as_ptr(), b.as_ptr(), 3);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = [1.0f32, 0.0];
        let b = [-1.0f32, 0.0];
        let sim = semantic_cosine(a.as_ptr(), b.as_ptr(), 2);
        assert!((sim + 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_null_returns_error() {
        let a = [1.0f32];
        let sim = semantic_cosine(a.as_ptr(), std::ptr::null(), 1);
        assert!((sim - (-2.0)).abs() < 0.001);
    }

    #[test]
    fn dim_before_init_is_zero() {
        // Don't call init — dim should be 0
        assert_eq!(semantic_dim(), 0);
    }
}
