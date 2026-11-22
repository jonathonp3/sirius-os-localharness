# Security

## Flatpak signing key

- Fingerprint: `65269CFC0D32C2CA632C48C5F8AF682DDA82A240`
- The private key is stored only as the `GPG_PRIVATE_KEY` secret in the
  protected `flatpak-signing` GitHub Environment. The job requires reviewer
  approval before the secret is released.
- The public key is committed at `packaging/flatpak/local-harness.gpg` and is
  published in the signed repository.
- Local exports of the private key must not exist in any repository, backup, or
  chat. After confirming the environment secret, destroy them (`shred -u`) or
  keep an offline, encrypted copy.

### Rotation

If the key is exposed or suspected compromised:

1. Revoke the old key and generate a new signing key.
2. Update the fingerprint in `.github/workflows/build.yml` and the public key at
   `packaging/flatpak/local-harness.gpg`.
3. Replace the `GPG_PRIVATE_KEY` environment secret.
4. Rebuild and republish; tell users to re-import the new `.flatpakrepo`.

## Reporting

Please use GitHub's private vulnerability reporting for this repository rather
than a public issue.
