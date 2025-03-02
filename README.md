# Ferron

Ferron is a work-in-progress web server written in Rust. It aims to be memory-safe, efficient, and highly customizable, making it a great choice for developers and administrators looking for a modern Rust-based server solution.

## Features (planned)

- **High performance**: Built with Rust’s async capabilities for optimal speed.
- **Memory-safe**: Built with Rust, which is a programming language offering memory safety.
- **Extensibility**: Modular architecture for easy customization.
- **Secure**: Focus on robust security practices and safe concurrency.

## Components

Ferron consists of multiple components:

- **`ferron`**: The main web server.
- **`ferron-common`**: A shared component used by `ferron` and its modules.
- **`ferron-passwd`**: A tool for generating user entries with hashed passwords, which can be copied into the web server's configuration file.
- **`ferron-mod-example`**: A dynamically linked module that can be loaded by `ferron` and responds with "Hello World!" for requests to the `/hello` URL.

## Installation

Since Ferron is still a work in progress, installation instructions will be provided once an initial release is available. Stay tuned!

## Getting started

For now, you can clone the repository and explore the existing code:

```sh
git clone https://github.com/ferronweb/ferron.git
cd ferron
```

You can build and run the project using Cargo:

```sh
cargo build -r
cargo run -r --bin ferron
```

## Server configuration

You can check the [Ferron documentation](https://www.ferronweb.org/docs/configuration) to see configuration properties used by Ferron.

## Contributing

Contributions are welcome! If you're interested in helping out, feel free to fork the repository, submit issues, or open pull requests.

### To contribute:
1. Fork the repository.
2. Create a new branch (`git checkout -b feature-branch`).
3. Make your changes and commit them (`git commit -m "feat: add new feature"`).
4. Push to your branch (`git push origin feature-branch`).
5. Open a pull request.

## Roadmap

- [x] Implement basic request handling
- [x] Support for static file serving
- [x] Middleware support
- [x] Logging and error handling improvements
- [x] HTTPS support
- [x] Support for CGI, FastCGI, and SCGI for dynamic content (via an optional built-in module)
- [x] Support for forward and reverse proxying (via an optional built-in module)
- [x] Support for caching (via an optional built-in module)
- [x] Load balancing support (via an optional built-in module)

## License

Ferron is licensed under the MIT License. See `LICENSE` for details.

---

Stay tuned for updates as development progresses!