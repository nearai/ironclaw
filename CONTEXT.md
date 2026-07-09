# IronClaw Reborn Domain Glossary

## Extension

The only installable product object. An extension has one stable `ExtensionId` and owns every tool, channel, auth, trigger, and file surface that it exposes.

## Extension package

The complete, immutable set of files installed for one extension version: its root manifest, manifest fragments, runtime artifacts, schemas, prompts, and other referenced assets.

## Root manifest

The sole installable manifest in an extension package. It owns extension identity, version, trust request, runtime selection, host-API membership, and the ordered list of any manifest fragments that form the extension contract.

## Manifest fragment

A typed, non-installable manifest document imported by the root manifest to declare one or more surfaces within a single host-API contract section. A fragment has no independent extension identity, version, trust, or runtime.

## Resolved extension manifest

The canonical, immutable manifest contract produced by resolving the root manifest and all of its declared fragments. Discovery, validation, trust evaluation, activation, runtime binding, lifecycle, and frontend projection consume this resolved contract rather than rereading source files.

## Capability surface

One product-facing face declared by an extension: tool, channel, auth, trigger, or file. Runtime kind is an implementation choice and is not a capability-surface kind.

## Surface key

The stable identity of one surface inside an extension, composed from its owning `ExtensionId`, surface kind, and manifest-local surface identifier.

## Surface binding

The runtime implementation bound to one declared surface. A binding may implement behavior but cannot add, remove, or widen the authority declared by its resolved manifest surface.

## Extension entrypoint

The single runtime boundary through which an extension supplies its surface bindings to the host. It binds implementations to an already-validated resolved manifest and does not redeclare the extension contract.

## Bound extension

An extension generation whose resolved manifest has been joined bijectively with runtime bindings and has passed trust, authority, and activation validation.

## Active generation

The immutable bound-extension snapshot currently serving an installation. Replacement is atomic: new work sees either the old complete generation or the new complete generation, never a partially rebound mixture.

## Adapter

A generic host-facing interface for one subsystem boundary, implemented by extension-specific runtime code. The host reasons about adapter contracts and normalized values, never a concrete product protocol.

## Channel adapter

The adapter family that extracts untrusted protocol hints, normalizes verified channel ingress, and renders normalized outbound communication. The host owns candidate selection and verification; protocol parsing, protocol identifiers, and vendor request rendering belong to the extension implementation.

## Provider

An external credential-account authority named by an auth surface. A `ProviderId` selects credential acquisition and account semantics; it is not an installable product identity and must not be used in place of `ExtensionId`.

## Host port

A capability-limited service supplied by the host to an extension binding, such as HTTP egress, secret access, persistence, clock, or turn submission. Sandboxed runtimes can exercise host authority only through host ports; native first-party extensions are trusted code and are required by dependency and source gates to use the same ports.

## Generic host

IronClaw runtime code that discovers, validates, activates, dispatches, and observes extensions using only resolved manifest data, surface keys, normalized subsystem contracts, and host ports. It contains no live behavior keyed to a concrete extension, channel, or provider name.
