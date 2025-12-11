#!/usr/bin/env python3
"""Embedding server using sentence-transformers MiniLM."""
from flask import Flask, request, jsonify
from sentence_transformers import SentenceTransformer

app = Flask(__name__)
model = SentenceTransformer("sentence-transformers/all-MiniLM-L6-v2")

@app.route("/health")
def health():
    return jsonify({"status": "ok"})

@app.route("/embed", methods=["POST"])
def embed():
    texts = request.json.get("inputs", [])
    if not texts:
        return jsonify([])
    return jsonify(model.encode(texts, convert_to_numpy=True, normalize_embeddings=True).tolist())

if __name__ == "__main__":
    app.run(host="0.0.0.0", port=8080)
