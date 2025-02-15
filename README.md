# Project Karpacz

Project Karpacz is a work-in-progress web server written in Rust. It aims to be memory-safe, efficient, and highly customizable, making it a great choice for developers and administrators looking for a modern Rust-based server solution.

## Features (planned)

- **High performance**: Built with Rust’s async capabilities for optimal speed.
- **Memory-safe**: Built with Rust, which is a programming language offering memory safety.
- **Extensibility**: Modular architecture for easy customization.
- **Secure**: Focus on robust security practices and safe concurrency.

## Components

Project Karpacz consists of multiple components:

- **`project-karpacz`**: The main web server.
- **`project-karpacz-common`**: A shared component used by `project-karpacz` and its modules.
- **`project-karpacz-passwd`**: A tool for generating user entries with hashed passwords, which can be copied into the web server's configuration file.
- **`project-karpacz-mod-example`**: A dynamically linked module that can be loaded by `project-karpacz` and responds with "Hello World!" for requests to the `/hello` URL.

## Installation

Since Project Karpacz is still a work in progress, installation instructions will be provided once an initial release is available. Stay tuned!

## Getting started

For now, you can clone the repository and explore the existing code:

```sh
git clone https://github.com/yourusername/project-karpacz.git
cd project-karpacz
```

You can build and run the project using Cargo:

```sh
cargo build -r
cargo run -r --bin project-karpacz
```

## Server configuration

You can check the `CONFIGURATION.md` file to see configuration properties used by Project Karpacz.

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
- [ ] Support for CGI, FastCGI, and SCGI for dynamic content (via a module)
- [ ] Support for forward and reverse proxying (via a module)
- [ ] Support for caching (via a module)
- [ ] Support for ModSecurity WAF (via a module that uses the "modsecurity" crate; source only)
- [ ] Load balancing support (via a module)

## License

Project Karpacz is licensed under the MIT License. See `LICENSE` for details.

---

Stay tuned for updates as development progresses!