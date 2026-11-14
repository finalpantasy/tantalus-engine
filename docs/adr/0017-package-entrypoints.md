# Packaged WASM entry points

Package manifests may declare named module entry points with a source path and ABI version. A package with an entry point need not contain declarative records, allowing executable-only modules.

`PackageLoader` applies the same canonical-root and parent/absolute-path rejection to modules as to content. Staging accepts only portable WASM binary headers, limits files to 16 MiB, retains bytes, and records SHA-256. Module names must be unique and use the supported ABI. The package content hash commits to content records plus sorted module names, digests, and ABI versions.

`ModuleHost::instantiate_entrypoint` selects a staged module by name and verifies that package version/content hash match the resolved `ProfileLock`. It also rechecks the module digest against its bytes before portable-WASM compilation. A real external action module has been staged, resolved, instantiated, and invoked through this path.

This boundary still accepts only portable WASM, not mod-supplied compiled native caches. WIT-generated bindings, module migration callbacks, full provider lifecycle, and useful-work performance evidence remain open.
