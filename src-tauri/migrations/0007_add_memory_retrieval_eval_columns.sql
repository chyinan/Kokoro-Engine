ALTER TABLE memory_retrieval_logs ADD COLUMN overlap_count INTEGER;
ALTER TABLE memory_retrieval_logs ADD COLUMN semantic_only_count INTEGER;
ALTER TABLE memory_retrieval_logs ADD COLUMN bm25_only_count INTEGER;
ALTER TABLE memory_retrieval_logs ADD COLUMN filtered_out_count INTEGER;
