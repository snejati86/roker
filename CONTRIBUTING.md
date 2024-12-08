# Contributing to Roker

Thank you for your interest in contributing to Roker! This document provides guidelines and instructions for contributing.

## Code of Conduct

This project adheres to the Rust Code of Conduct. By participating, you are expected to uphold this code.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/your-username/roker.git`
3. Create a branch: `git checkout -b your-feature-name`
4. Make your changes
5. Run tests: `cargo test`
6. Run benchmarks: `cargo bench`
7. Commit your changes: `git commit -m "Description of changes"`
8. Push to your fork: `git push origin your-feature-name`
9. Open a Pull Request

## Development Guidelines

### Code Style

- Follow the Rust style guide
- Use `cargo fmt` before committing
- Run `cargo clippy` and address any warnings
- Add comments for complex logic
- Update documentation when changing public APIs

### Testing

- Add tests for new features
- Ensure all tests pass: `cargo test`
- Add benchmarks for performance-critical code
- Test on different platforms if possible

### Commit Messages

- Use clear and descriptive commit messages
- Start with a verb in imperative mood (e.g., "Add", "Fix", "Update")
- Reference issues and pull requests where appropriate

### Documentation

- Update README.md for significant changes
- Add rustdoc comments for public APIs
- Include examples in documentation
- Update CHANGELOG.md

## Pull Request Process

1. Update documentation
2. Add tests for new features
3. Ensure CI passes
4. Request review from maintainers
5. Address review feedback
6. Update based on feedback

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run benchmarks
cargo bench

# Run with logging
RUST_LOG=debug cargo test
```

## Common Tasks

### Adding a New Feature

1. Create an issue describing the feature
2. Discuss implementation approach
3. Implement the feature
4. Add tests and documentation
5. Submit a pull request

### Fixing a Bug

1. Create an issue describing the bug
2. Add a test case that reproduces the bug
3. Fix the bug
4. Verify the test passes
5. Submit a pull request

## Getting Help

- Open an issue for questions
- Join our Discord server
- Check existing issues and pull requests

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project (MIT/Apache-2.0). 