# Use the official Node.js image
FROM node:22-alpine

# Set the working directory
WORKDIR /app

# Copy files into the container
COPY . .

# Install dependencies
RUN npm install

# Expose port 3000
EXPOSE 3000

# Start the application
CMD ["node", "index.js"]
