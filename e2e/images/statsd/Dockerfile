FROM python:3.14-slim
WORKDIR /app
COPY responder.py /app/responder.py
COPY requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir -r /app/requirements.txt
EXPOSE 8125
EXPOSE 8080
CMD ["python3", "-u", "responder.py"]
