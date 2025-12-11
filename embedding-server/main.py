#!/usr/bin/env python3
"""
Fast embedding server using sentence-transformers with ONNX optimization.
Uses all-MiniLM-L6-v2 (fast, 22M params) with ONNX Runtime for ~2x speedup.
"""
import os
from flask import Flask, request, jsonify
import numpy as np

app = Flask(__name__)

MODEL_NAME = os.environ.get("MODEL_NAME", "sentence-transformers/all-MiniLM-L6-v2")
USE_ONNX = os.environ.get("USE_ONNX", "true").lower() == "true"

print(f"Loading model: {MODEL_NAME}")
print(f"ONNX optimization: {USE_ONNX}")

if USE_ONNX:
    from optimum.onnxruntime import ORTModelForFeatureExtraction
    from transformers import AutoTokenizer
    import torch

    tokenizer = AutoTokenizer.from_pretrained(MODEL_NAME)
    model = ORTModelForFeatureExtraction.from_pretrained(MODEL_NAME, export=True)

    def encode(texts):
        inputs = tokenizer(texts, padding=True, truncation=True, max_length=512, return_tensors="pt")
        with torch.no_grad():
            outputs = model(**inputs)
        # Mean pooling
        attention_mask = inputs["attention_mask"]
        token_embeddings = outputs.last_hidden_state
        input_mask_expanded = attention_mask.unsqueeze(-1).expand(token_embeddings.size()).float()
        embeddings = torch.sum(token_embeddings * input_mask_expanded, 1) / torch.clamp(input_mask_expanded.sum(1), min=1e-9)
        # Normalize
        embeddings = torch.nn.functional.normalize(embeddings, p=2, dim=1)
        return embeddings.numpy()
else:
    from sentence_transformers import SentenceTransformer
    model = SentenceTransformer(MODEL_NAME)

    def encode(texts):
        return model.encode(texts, convert_to_numpy=True, normalize_embeddings=True)

print("Model loaded!")

@app.route("/health", methods=["GET"])
def health():
    return jsonify({"status": "ok", "model": MODEL_NAME, "onnx": USE_ONNX})

@app.route("/embed", methods=["POST"])
def embed():
    data = request.json
    texts = data.get("inputs", [])

    if not texts:
        return jsonify([])

    embeddings = encode(texts)
    return jsonify(embeddings.tolist())

if __name__ == "__main__":
    port = int(os.environ.get("PORT", 8080))
    app.run(host="0.0.0.0", port=port, threaded=True)
