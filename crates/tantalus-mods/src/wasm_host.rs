use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::ProfileLock;
use wasmtime::{
    Config, Engine, Func, FuncType, Instance, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc, ValType,
};

pub const MODULE_ABI_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleLimits {
    pub fuel: u64,
    pub max_memory_bytes: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_commands: usize,
    pub max_stack_bytes: usize,
}

impl Default for ModuleLimits {
    fn default() -> Self {
        Self {
            fuel: 100_000,
            max_memory_bytes: 2 * 1024 * 1024,
            max_input_bytes: 64 * 1024,
            max_output_bytes: 1024 * 1024,
            max_commands: 1_024,
            max_stack_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleErrorKind {
    InvalidModule,
    MissingExport,
    AbiMismatch,
    Trap,
    FuelExhausted,
    ResourceLimit,
    HostValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleError {
    pub kind: ModuleErrorKind,
    pub package: String,
    pub context: String,
}

impl ModuleError {
    fn new(kind: ModuleErrorKind, package: &str, context: impl Into<String>) -> Self {
        Self {
            kind,
            package: package.to_owned(),
            context: context.into(),
        }
    }
}

impl Display for ModuleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "module {}: {}", self.package, self.context)
    }
}

impl Error for ModuleError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleEvent {
    pub name: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleInvocation {
    pub module_state: serde_json::Value,
    pub events: Vec<ModuleEvent>,
    pub fuel_consumed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HostCommand {
    SetState {
        key: String,
        value: serde_json::Value,
    },
    AllocateId,
    EmitEvent {
        name: String,
        data: serde_json::Value,
    },
}

#[derive(Debug)]
struct HostState {
    limits: StoreLimits,
    max_commands: usize,
    commands: Vec<HostCommand>,
    failure: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GuestOutput {
    state: serde_json::Value,
    #[serde(default)]
    events: Vec<ModuleEvent>,
}

pub struct ModuleHost {
    package: String,
    engine: Engine,
    module: Module,
    limits: ModuleLimits,
    module_state: serde_json::Value,
    authoritative_state: BTreeMap<String, serde_json::Value>,
    next_id: u64,
}

impl ModuleHost {
    pub fn instantiate(
        lock: &ProfileLock,
        package: &str,
        wasm: &[u8],
        limits: ModuleLimits,
    ) -> Result<Self, ModuleError> {
        if !lock.modules.iter().any(|module| module.package == package) {
            return Err(ModuleError::new(
                ModuleErrorKind::HostValidation,
                package,
                "package is not present in the resolved profile lock",
            ));
        }
        if limits.fuel == 0
            || limits.max_memory_bytes < 64 * 1024
            || limits.max_input_bytes == 0
            || limits.max_output_bytes == 0
            || limits.max_stack_bytes == 0
        {
            return Err(ModuleError::new(
                ModuleErrorKind::ResourceLimit,
                package,
                "fuel, stack, input, output, and viable memory limits must be nonzero",
            ));
        }
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .cranelift_nan_canonicalization(true)
            .relaxed_simd_deterministic(true)
            .wasm_relaxed_simd(false)
            .wasm_multi_memory(false)
            .max_wasm_stack(limits.max_stack_bytes);
        let engine = Engine::new(&config).map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::InvalidModule,
                package,
                format!("failed to configure deterministic engine: {error}"),
            )
        })?;
        let module = Module::new(&engine, wasm).map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::InvalidModule,
                package,
                format!("portable WASM validation/compilation failed: {error}"),
            )
        })?;
        Ok(Self {
            package: package.to_owned(),
            engine,
            module,
            limits,
            module_state: serde_json::Value::Null,
            authoritative_state: BTreeMap::new(),
            next_id: 1,
        })
    }

    pub fn wasm_sha256(wasm: &[u8]) -> String {
        format!("{:x}", Sha256::digest(wasm))
    }

    pub fn module_state(&self) -> &serde_json::Value {
        &self.module_state
    }

    pub fn authoritative_state(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.authoritative_state
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    pub fn invoke(&mut self, input: &serde_json::Value) -> Result<ModuleInvocation, ModuleError> {
        let request = serde_json::to_vec(&serde_json::json!({
            "abi_version": MODULE_ABI_VERSION,
            "module_state": self.module_state,
            "input": input,
        }))
        .map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::HostValidation,
                &self.package,
                error.to_string(),
            )
        })?;
        if request.len() > self.limits.max_input_bytes {
            return Err(ModuleError::new(
                ModuleErrorKind::ResourceLimit,
                &self.package,
                format!(
                    "input is {} bytes, exceeding the {} byte limit",
                    request.len(),
                    self.limits.max_input_bytes
                ),
            ));
        }

        let host_state = HostState {
            limits: StoreLimitsBuilder::new()
                .memory_size(self.limits.max_memory_bytes)
                .table_elements(10_000)
                .instances(1)
                .tables(1)
                .memories(1)
                .build(),
            max_commands: self.limits.max_commands,
            commands: Vec::new(),
            failure: None,
        };
        let mut store = Store::new(&self.engine, host_state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.limits.fuel).map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::ResourceLimit,
                &self.package,
                error.to_string(),
            )
        })?;
        let max_command_bytes = self.limits.max_input_bytes;
        let emit = Func::new(
            &mut store,
            FuncType::new(&self.engine, [ValType::I32, ValType::I32], []),
            move |mut caller: wasmtime::Caller<'_, HostState>, inputs, _results| {
                let (Some(wasmtime::Val::I32(pointer)), Some(wasmtime::Val::I32(length))) =
                    (inputs.first(), inputs.get(1))
                else {
                    caller.data_mut().failure =
                        Some("host import received invalid arguments".to_owned());
                    return Ok(());
                };
                let (pointer, length) = (*pointer, *length);
                if caller.data().failure.is_some() {
                    return Ok(());
                }
                if caller.data().commands.len() >= caller.data().max_commands {
                    caller.data_mut().failure = Some("host command limit exceeded".to_owned());
                    return Ok(());
                }
                let Some(memory) = caller
                    .get_export("memory")
                    .and_then(|export| export.into_memory())
                else {
                    caller.data_mut().failure =
                        Some("guest memory is unavailable during host import".to_owned());
                    return Ok(());
                };
                let bytes = match guest_bytes(&memory, &caller, pointer, length, max_command_bytes)
                {
                    Ok(bytes) => bytes.to_vec(),
                    Err(error) => {
                        caller.data_mut().failure = Some(error);
                        return Ok(());
                    }
                };
                match serde_json::from_slice::<HostCommand>(&bytes) {
                    Ok(command) => caller.data_mut().commands.push(command),
                    Err(error) => {
                        caller.data_mut().failure = Some(format!("invalid host command: {error}"));
                    }
                }
                Ok(())
            },
        );
        let mut linker = wasmtime::Linker::new(&self.engine);
        linker
            .define(&mut store, "tantalus", "emit", emit)
            .map_err(|error| {
                ModuleError::new(
                    ModuleErrorKind::InvalidModule,
                    &self.package,
                    error.to_string(),
                )
            })?;
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|error| {
                ModuleError::new(
                    ModuleErrorKind::InvalidModule,
                    &self.package,
                    format!("instantiation failed: {error}"),
                )
            })?;
        let memory = export_memory(&instance, &mut store, &self.package)?;
        let abi: TypedFunc<(), i32> =
            typed_export(&instance, &mut store, "tantalus_abi_version", &self.package)?;
        let version = abi.call(&mut store, ()).map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::Trap,
                &self.package,
                format!("ABI version callback trapped: {error}"),
            )
        })?;
        if version != MODULE_ABI_VERSION {
            return Err(ModuleError::new(
                ModuleErrorKind::AbiMismatch,
                &self.package,
                format!("module ABI {version} does not match host ABI {MODULE_ABI_VERSION}"),
            ));
        }
        let alloc: TypedFunc<i32, i32> =
            typed_export(&instance, &mut store, "alloc", &self.package)?;
        let invoke: TypedFunc<i64, i64> =
            typed_export(&instance, &mut store, "tantalus_invoke", &self.package)?;
        let input_length = i32::try_from(request.len()).map_err(|_| {
            ModuleError::new(
                ModuleErrorKind::ResourceLimit,
                &self.package,
                "input exceeds guest ABI range",
            )
        })?;
        let pointer = alloc
            .call(&mut store, input_length)
            .map_err(|error| self.trap_error("guest allocation", error))?;
        write_guest(&memory, &mut store, pointer, &request, &self.package)?;
        let input = (i64::from(input_length) << 32) | i64::from(pointer);
        let packed = invoke
            .call(&mut store, input)
            .map_err(|error| self.trap_error("module callback", error))?;
        if let Some(failure) = store.data().failure.clone() {
            return Err(ModuleError::new(
                ModuleErrorKind::HostValidation,
                &self.package,
                format!("transaction rolled back: {failure}"),
            ));
        }
        let output_pointer = packed as u64 as u32;
        let output_length = (packed as u64 >> 32) as u32;
        if output_length as usize > self.limits.max_output_bytes {
            return Err(ModuleError::new(
                ModuleErrorKind::ResourceLimit,
                &self.package,
                "guest output exceeds its declared limit",
            ));
        }
        let output = guest_bytes(
            &memory,
            &store,
            i32::try_from(output_pointer).unwrap_or(-1),
            i32::try_from(output_length).unwrap_or(-1),
            self.limits.max_output_bytes,
        )
        .map_err(|error| ModuleError::new(ModuleErrorKind::ResourceLimit, &self.package, error))?;
        let output: GuestOutput = serde_json::from_slice(output).map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::HostValidation,
                &self.package,
                format!("invalid guest output; transaction rolled back: {error}"),
            )
        })?;

        let mut staged_state = self.authoritative_state.clone();
        let mut staged_next_id = self.next_id;
        let mut events = output.events.clone();
        for command in &store.data().commands {
            match command {
                HostCommand::SetState { key, value } => {
                    if key.is_empty() || key.starts_with("internal:") {
                        return Err(ModuleError::new(
                            ModuleErrorKind::HostValidation,
                            &self.package,
                            "state keys must be nonempty and cannot use the internal namespace",
                        ));
                    }
                    staged_state.insert(key.clone(), value.clone());
                }
                HostCommand::AllocateId => {
                    staged_next_id = staged_next_id.checked_add(1).ok_or_else(|| {
                        ModuleError::new(
                            ModuleErrorKind::ResourceLimit,
                            &self.package,
                            "host ID space exhausted",
                        )
                    })?;
                }
                HostCommand::EmitEvent { name, data } => {
                    if name.is_empty() {
                        return Err(ModuleError::new(
                            ModuleErrorKind::HostValidation,
                            &self.package,
                            "event names must be nonempty",
                        ));
                    }
                    events.push(ModuleEvent {
                        name: name.clone(),
                        data: data.clone(),
                    });
                }
            }
        }
        let fuel_consumed = self.limits.fuel
            - store.get_fuel().map_err(|error| {
                ModuleError::new(
                    ModuleErrorKind::ResourceLimit,
                    &self.package,
                    error.to_string(),
                )
            })?;
        self.module_state = output.state;
        self.authoritative_state = staged_state;
        self.next_id = staged_next_id;
        Ok(ModuleInvocation {
            module_state: self.module_state.clone(),
            events,
            fuel_consumed,
        })
    }

    fn trap_error(&self, operation: &str, error: wasmtime::Error) -> ModuleError {
        let text = error.to_string();
        let chain = error
            .chain()
            .map(|cause| cause.to_string().to_lowercase())
            .collect::<Vec<_>>()
            .join("; ");
        let kind = if chain.contains("fuel") {
            ModuleErrorKind::FuelExhausted
        } else if chain.contains("memory") || chain.contains("table") {
            ModuleErrorKind::ResourceLimit
        } else {
            ModuleErrorKind::Trap
        };
        ModuleError::new(
            kind,
            &self.package,
            format!("{operation} failed; transaction rolled back: {text}"),
        )
    }
}

fn typed_export<Params, Results>(
    instance: &Instance,
    store: &mut Store<HostState>,
    name: &str,
    package: &str,
) -> Result<TypedFunc<Params, Results>, ModuleError>
where
    Params: wasmtime::WasmParams,
    Results: wasmtime::WasmResults,
{
    instance
        .get_typed_func::<Params, Results>(store, name)
        .map_err(|error| {
            ModuleError::new(
                ModuleErrorKind::MissingExport,
                package,
                format!("required export `{name}` is missing or has the wrong type: {error}"),
            )
        })
}

fn export_memory(
    instance: &Instance,
    store: &mut Store<HostState>,
    package: &str,
) -> Result<Memory, ModuleError> {
    instance
        .get_export(store, "memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| {
            ModuleError::new(
                ModuleErrorKind::MissingExport,
                package,
                "required linear-memory export `memory` is missing",
            )
        })
}

fn guest_bytes<'a>(
    memory: &'a Memory,
    store: &'a impl wasmtime::AsContext<Data = HostState>,
    pointer: i32,
    length: i32,
    maximum: usize,
) -> Result<&'a [u8], String> {
    let pointer = usize::try_from(pointer).map_err(|_| "negative guest pointer".to_owned())?;
    let length = usize::try_from(length).map_err(|_| "negative guest length".to_owned())?;
    if length > maximum {
        return Err("guest buffer exceeds its limit".to_owned());
    }
    let end = pointer
        .checked_add(length)
        .ok_or_else(|| "guest buffer range overflow".to_owned())?;
    memory
        .data(store)
        .get(pointer..end)
        .ok_or_else(|| "guest buffer is outside linear memory".to_owned())
}

fn write_guest(
    memory: &Memory,
    store: &mut Store<HostState>,
    pointer: i32,
    bytes: &[u8],
    package: &str,
) -> Result<(), ModuleError> {
    let pointer = usize::try_from(pointer).map_err(|_| {
        ModuleError::new(
            ModuleErrorKind::ResourceLimit,
            package,
            "guest returned a negative allocation pointer",
        )
    })?;
    let end = pointer.checked_add(bytes.len()).ok_or_else(|| {
        ModuleError::new(
            ModuleErrorKind::ResourceLimit,
            package,
            "guest allocation range overflow",
        )
    })?;
    let destination = memory
        .data_mut(store)
        .get_mut(pointer..end)
        .ok_or_else(|| {
            ModuleError::new(
                ModuleErrorKind::ResourceLimit,
                package,
                "guest allocation is outside linear memory",
            )
        })?;
    destination.copy_from_slice(bytes);
    Ok(())
}
