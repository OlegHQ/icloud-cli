# Hide My Email (iCloud+)

Apple does not publish a public API for **Hide My Email**. Third-party tools call the same private endpoints as **icloud.com**.

## Primary reference

- **[glizzykingdreko/icloud-hme](https://github.com/glizzykingdreko/icloud-hme)** (Python)  
  - **SRP** (Secure Remote Password) authentication with **2FA** support.  
  - Create/manage aliases; documents **rate limits** (order of ~5 per 30 minutes per account tier; total alias caps).  
  - **Rust port goal:** Reproduce login + alias HTTP flows; use well-audited crypto crates for SRP as required by Apple’s idmsa flow.

## User-facing documentation (behavior only)

- [Create and manage Hide My Email (iCloud user guide)](https://support.apple.com/guide/icloud/mm1a876f7aed/icloud) — product behavior, not API.

## Implementation notes

- Surface in CLI as `icloud hme list|create|deactivate|...` with **rate-limit** hints in `--help`.
- Never log alias email addresses in tracing unless log level and redaction policy explicitly allow (default: don’t).
