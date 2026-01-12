=== "Recommended: install to PATH"

    ```bash
    make install
    incan --version
    ```

    Notes:

    - `make install` installs `incan` to `~/.cargo/bin`.
    - If `incan` is not found, make sure `~/.cargo/bin` is on your `PATH`.

=== "Fallback: no install"

    ```bash
    make release
    ./target/release/incan --version
    ```
