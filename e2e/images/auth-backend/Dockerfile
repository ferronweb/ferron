FROM node:24-alpine

WORKDIR /app

COPY package.json ./
RUN npm install

COPY index.js ./

EXPOSE 9090

CMD ["node", "index.js"]
