// stub

#[cfg(test)]
mod tests {
    /// Proves sherpa-rs (and its native sherpa-onnx libs) link into the test binary.
    #[test]
    fn sherpa_links() {
        let cfg = sherpa_rs::transducer::TransducerConfig::default();
        assert!(cfg.encoder.is_empty());
        // Calls into native sherpa-onnx, so the linker must resolve it. With no model
        // files configured it is expected to fail gracefully rather than succeed.
        let r = sherpa_rs::transducer::TransducerRecognizer::new(cfg);
        assert!(r.is_err());
    }
}
