FROM python:3.12-slim
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt && useradd --uid 10001 --create-home relay && mkdir /data && chown relay:relay /data
COPY server.py .
USER relay
VOLUME /data
EXPOSE 8080
CMD ["python", "server.py"]
