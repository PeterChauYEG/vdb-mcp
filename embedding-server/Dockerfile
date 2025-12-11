FROM python:3.11-slim

WORKDIR /app

# Install PyTorch CPU version first
RUN pip install --no-cache-dir torch --index-url https://download.pytorch.org/whl/cpu

# Install sentence-transformers and gunicorn for production
RUN pip install --no-cache-dir flask sentence-transformers gunicorn

COPY main.py .

# Pre-download the model during build
RUN python -c "from sentence_transformers import SentenceTransformer; SentenceTransformer('sentence-transformers/all-MiniLM-L6-v2')"

EXPOSE 8080

# Use gunicorn with preload to share model across workers
CMD ["gunicorn", "--bind", "0.0.0.0:8080", "--workers", "2", "--threads", "4", "--preload", "main:app"]
