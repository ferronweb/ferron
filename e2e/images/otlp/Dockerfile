FROM python:3.11-slim
WORKDIR /app
COPY responder.py /app/responder.py
RUN pip install --no-cache-dir flask
EXPOSE 4318
CMD ["python3", "responder.py"]
