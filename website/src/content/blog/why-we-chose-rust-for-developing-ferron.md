---
title: Why we chose Rust for developing Ferron?
description: Discover why we chose Rust for Ferron, a fast, memory-safe web server, highlighting its performance, safety, and concurrency benefits.
date: 2025-03-06 15:23:00
cover: /img/covers/why-we-chose-rust-for-developing-ferron.png
---

Choosing the right programming language for web server development is very important, as it affects performance, security, scalability, and maintainability. It determines the server's ability to handle traffic efficiently and reliably.

In this post, we'll explain why we chose Rust to develop Ferron, a fast, memory-safe web server.

## Performance without compromise

One of the main reasons we chose Rust is its high performance. Rust offers low-level control similar to C and C++ while avoiding the overhead of garbage collection. With zero-cost abstractions, Rust enables Ferron to execute tasks with minimal latency, making it well-suited for high-performance web applications.

## Memory safety without a garbage collector

Web servers must be highly reliable, and memory safety issues like buffer overflows or use-after-free errors can lead to crashes or security vulnerabilities. Rust enforces memory safety at compile time, preventing many such bugs without relying on a garbage collector. This allows Ferron to achieve both safety and efficiency while avoiding garbage collection overhead.

## Concurrency without data races

Modern web servers must efficiently handle multiple concurrent connections. Rust’s ownership rules and fearless concurrency model enable Ferron to execute concurrent tasks without the risk of data races, ensuring stability even under high loads.

## Ecosystem and developer productivity

Rust has a growing ecosystem with powerful libraries like Hyper and Tokio (both of which we use in Ferron) that simplify building fast web servers. The Cargo package manager streamlines dependency management, while Rust's strong type system reduces runtime errors, enhancing the development experience.

## Evaluating other programming languages

We developed Ferron after discovering that SVR.JS - a web server built on Node.js by the same developer - was too slow in performance benchmarks.

Seeking better performance, the creator of SVR.JS considered a systems programming language. JavaScript and other interpreted or JIT-compiled languages were ruled out due to their insufficient speed for this use case.

We tested "Hello, World" web applications in various languages, including C, C++, and Rust. The results showed that a Rust-based web server using Tokio and Hyper was optimal for hosting websites.

- **C** - while highly performant, C lacks built-in memory safety, making web servers written in C vulnerable to security issues like buffer overflows.
- **C++** - although more feature-rich than C, C++ requires manual memory management, increasing the risk of stability and security vulnerabilities.
- **Go** - Go offers strong concurrency support and simplicity, but its garbage collection can introduce unpredictable latency.

Rust stood out for its unique combination of performance, memory safety, and concurrency - without the overhead of garbage collection. This ensures both speed and reliability, making it an ideal choice for building a modern web server like Ferron.

## Conclusion

In conclusion, the decision to use Rust for developing Ferron, a fast web server, was driven by its unique combination of performance, memory safety, and concurrency support. Rust's ability to deliver low-level control without the overhead of garbage collection, along with its robust ecosystem and developer-friendly features, makes it an optimal choice for modern web server development. By choosing Rust, we ensure that Ferron can handle traffic efficiently and reliably, providing a secure and scalable solution for web applications.