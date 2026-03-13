## Testing

- Use `cargo test` to run all tests in this workspace
- To run the full suite of tests CI will run, see `scripts/ci-local.py`.
- Before commiting run `cargo fmt` and `cargo clippy`, as well as `cd decoder && cargo fmt && cargo clippy`. YOU MUST FIX CLIPPY ERRORS.
  
## Finishing Up
When instructed to "finish up", follow this process:

1. Run `cargo +nightly fmt --all` to format all code
2. Run `cargo clippy --workspace --all-features -- -D warnings` and fix all errors
3. Fix any small issues found. For big issues, confirm with the user first
4. Amend changes into the most recent commit: `git add -A && git commit --amend --no-edit`
5. Push: `git push --force-with-lease`
6. Monitor CI on GitHub to ensure it passes. If it fails:
   - Analyze why the issue was missed
   - Update the pre-CI checks to catch similar issues
   - Fix the problem and repeat from step 1

Note: For a full local CI check matching what runs on GitHub, use `scripts/ci-local.py`

Helpful commands for monitoring CI:
- Check PR status: `gh pr checks <branch-name>`
- Watch until completion: `gh pr checks <branch-name> --watch` (refreshes every 10s, exits when done)
- View with details: `gh pr view <branch-name> --json statusCheckRollup --jq '.statusCheckRollup[] | "\(.name): \(.status) - \(.conclusion)"'`

