FROM node:20-alpine

WORKDIR /app

COPY package.json ./
RUN npm install

COPY index.js ./

EXPOSE 9090

CMD ["node", "index.js"]
