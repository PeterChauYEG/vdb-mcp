#!/usr/bin/env python3
"""
Fast embedding server using sentence-transformers with ONNX optimization.
"""
import os
from flask import Flask, request, jsonify
from sentence_transformers import SentenceTransformer

app = Flask(__name__)

# Use small, fast model - all-MiniLM-L6-v2 is good balance
# For even faster: paraphrase-MiniLM-L3-v2 (3 layers vs 6)
MODEL_NAME = os.environ.get("MODEL_NAME", "sentence-transformers/all-MiniLM-L6-v2")

print(f"Loading model: {MODEL_NAME}")
model = SentenceTransformer(MODEL_NAME)
print("Model loaded!")

@app.route("/health", methods=["GET"])
def health():
    return jsonify({"status": "ok"})

@app.route("/embed", methods=["POST"])
def embed():
    data = request.json
    texts = data.get("inputs", [])

    if not texts:
        return jsonify([])

    # Generate embeddings - sentence-transformers handles batching efficiently
    embeddings = model.encode(texts, convert_to_numpy=True, normalize_embeddings=True)

    return jsonify(embeddings.tolist())

if __name__ == "__main__":
    port = int(os.environ.get("PORT", 8080))
    app.run(host="0.0.0.0", port=port, threaded=True)
