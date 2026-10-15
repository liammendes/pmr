# Security

For a suspected vulnerability, email hello@liammendes.co.uk with `pmr security` in the subject. Include affected versions, reproduction steps and likely impact. Please allow time to investigate before publishing exploit details. There is no guaranteed response time or bug bounty.

This early release has not received an independent security audit. Fixes target the latest release; older releases do not have a separate maintenance commitment.

PMR can execute commands with the privileges of its user. Treat ecosystem files, modules and deployment hooks as executable code. Use a dedicated service account where appropriate, keep manager directories private and do not expose administrative endpoints to an untrusted network. Process dumps and diagnostic output can contain environment secrets.
