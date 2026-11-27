# Security Policy

## Supported versions

Security fixes go into the latest release. Please upgrade before reporting an issue you found on an older version.

## Reporting a vulnerability

Do not open a public issue for a vulnerability. Report it privately through GitHub's **Security > Report a vulnerability** form for this repository.

Include what you found, how to reproduce it, and the ReToken version (`retoken --version`) and platform.

## Scope

ReToken is a local proxy that handles API credentials and source code, so these areas are in scope:

- Anything that lets a non-local party reach the proxy or its `/_retoken/*` admin routes (for example DNS rebinding).
- Server-side request forgery through custom upstream URLs.
- Credentials or request content leaking across cache keys, into logs, or to any host other than the configured provider.
- Unbounded memory or disk use from crafted requests or upstream responses.
