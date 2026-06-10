<p align="center">
  <!-- <img src="https://github.com/sputnikmd/sputnik/blob/main/assets/logo.png"> -->
  <img src="./assets/Logo.png">
</p>

<h1 align="center">Sputnik</h1>

> [!WARNING]
> This project is still under active development. Expect bugs, breaking changes, and incomplete features.

# 🛠 Compilation

```bash
git clone https://github.com/sputnikmd/sputnik.git
cd sputnik
cargo build --release
```

## 💻 Development

This project uses [`just`](https://github.com/casey/just) as a command runner for common tasks.
If you have `just` installed, you can simply run:

```bash
just         # Lists all available commands
just run     # Runs the application in debug mode
just build   # Builds the application
just test    # Runs the test suite
just fmt     # Formats the codebase
just clippy  # Runs the linter
```

### Using Nix

If you are using Nix, you can enter the development shell which has `just` and all necessary dependencies pre-installed:

```bash
nix develop
just run
```
