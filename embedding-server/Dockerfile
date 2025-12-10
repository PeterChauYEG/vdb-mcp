FROM python:3.11-slim

WORKDIR /app

# Install PyTorch CPU version first
RUN pip install --no-cache-dir torch --index-url https://download.pytorch.org/whl/cpu

# Install other dependencies from PyPI
RUN pip install --no-cache-dir flask sentence-transformers

COPY main.py .

# Pre-download the model during build
RUN python -c "from sentence_transformers import SentenceTransformer; SentenceTransformer('sentence-transformers/all-MiniLM-L6-v2')"

EXPOSE 8080

CMD ["python", "main.py"]
