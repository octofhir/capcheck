# Changelog

## [0.1.1] - 2026-11-06

Fewer false alarms, found on a server with a large statement:

- An operation in `rest.operation` that is missing at system level only fails when its
  OperationDefinition has `system = true`: that list also holds operations of several resource
  types.
- A search result with no `self` link no longer counts as an ignored parameter.
- A 500 on an unknown parameter counts as refusing it.

## [0.1.0] - 2026-11-06

First release: interactions, versioning, conditional reads, search parameters, includes and
operations of a CapabilityStatement checked against a running server; writes on Patient with
`--write`; text, Markdown and JSON reports; exit codes for CI.
