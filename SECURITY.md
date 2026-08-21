# Security Policy

## Supported Versions

Only the latest release is actively supported with security updates.

| Version | Supported          |
| ------- | ------------------ |
| latest  | :white_check_mark: |
| < latest | :x:                |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue
in Nuphus, please report it responsibly.

### Report Channels

- **GitHub Security Advisory**: [https://github.com/mrpulor-gh/nuphus/security/advisories/new](https://github.com/mrpulor-gh/nuphus/security/advisories/new)

### Response Commitment

- **Initial Response**: Within 48 hours (business days)
- **Status Update**: At least every 5 business days until resolution
- **Disclosure**: We aim to publish advisories within 90 days, or earlier if
  coordinated with the reporter

### What to Include

When reporting, please provide as much of the following as possible:

- Description of the vulnerability
- Steps to reproduce
- Affected versions
- Potential impact
- Suggested mitigations or fixes (if any)

### Scope

The following are in scope for our security program:

- The Nuphus desktop application (Tauri shell, Rust backend)
- The Nuphus web frontend (React 18 + TypeScript)
- CLI tooling shipped with Nuphus
- Plugin execution sandbox and permission model

The following are generally out of scope:

- Issues in dependencies unless they directly affect Nuphus in a novel way
- Theoretical attacks requiring physical access to the user's machine
- Social engineering attacks
- Denial of service via resource exhaustion on a local machine
- Issues in user-authored plugins or custom workflows

### Safe Harbor

We will not pursue legal action against researchers who:

- Make a good faith effort to avoid privacy violations, data destruction, or
  service disruption
- Report vulnerabilities promptly through the channels listed above
- Provide a reasonable time for us to address the issue before any public
  disclosure

## Security Best Practices for Users

- Always run the latest version of Nuphus
- Review plugin permissions before installation
- Keep your operating system and dependencies up to date
- Do not run Nuphus with elevated privileges unless absolutely necessary