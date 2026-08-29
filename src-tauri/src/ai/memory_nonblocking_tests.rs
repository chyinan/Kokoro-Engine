// pattern: Functional Core

#[cfg(test)]
mod tests {
    use super::super::memory::{memory_retrieval_mode, MemoryRetrievalMode};
    use super::super::memory_embedding_model::{
        memory_embedding_model_availability, MemoryEmbeddingModelAvailability,
        MemoryEmbeddingModelStatus,
    };

    fn status(installed: bool, missing_files: &[&str]) -> MemoryEmbeddingModelStatus {
        MemoryEmbeddingModelStatus {
            installed,
            repo_id: "test/repo".to_string(),
            download_url: "https://example.invalid/model".to_string(),
            install_dir: "".to_string(),
            model_path: "".to_string(),
            required_files: vec!["model.onnx".to_string()],
            missing_files: missing_files
                .iter()
                .map(|file| (*file).to_string())
                .collect(),
        }
    }

    #[test]
    fn missing_model_is_typed_unavailable() {
        assert_eq!(
            memory_embedding_model_availability(&status(false, &["model.onnx"])),
            MemoryEmbeddingModelAvailability::Unavailable
        );
    }

    #[test]
    fn ready_model_allows_semantic_retrieval() {
        assert_eq!(
            memory_embedding_model_availability(&status(true, &[])),
            MemoryEmbeddingModelAvailability::Ready
        );
        assert_eq!(
            memory_retrieval_mode(MemoryEmbeddingModelAvailability::Ready),
            MemoryRetrievalMode::Ready
        );
    }

    #[test]
    fn unavailable_model_bypasses_semantic_retrieval_with_empty_result() {
        assert_eq!(
            memory_retrieval_mode(MemoryEmbeddingModelAvailability::Unavailable),
            MemoryRetrievalMode::Unavailable
        );
    }
}
