# Repli auto-hébergé (ex. Oracle Cloud Always Free, ARM) : docker build -t trailopt . && docker run -p 8501:8501 trailopt
FROM python:3.12-slim
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PIP_NO_CACHE_DIR=1 \
    TRAILOPT_CACHE_DIR=/tmp/trailopt_cache
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends libexpat1 \
    && rm -rf /var/lib/apt/lists/*
COPY requirements.txt .
RUN pip install -r requirements.txt
COPY trailopt ./trailopt
COPY app.py .
COPY ui ./ui
RUN useradd -m app && chown -R app /app
USER app
EXPOSE 8501
HEALTHCHECK CMD python -c "import urllib.request; urllib.request.urlopen('http://localhost:8501/_stcore/health')"
CMD ["streamlit", "run", "app.py", "--server.port=8501", "--server.address=0.0.0.0", "--server.headless=true", "--browser.gatherUsageStats=false"]
