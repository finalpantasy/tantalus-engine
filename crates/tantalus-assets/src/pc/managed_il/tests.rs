use super::*;

struct Fake;

const CTOR_STAT: u32 = 0x0600_0001;
const CTOR_DICT: u32 = 0x0A00_0001;
const CALL_ADD: u32 = 0x0A00_0002;
const CALL_INIT: u32 = 0x0A00_0003;
const FIELD_A: u32 = 0x0400_0001;
const FIELD_BLOB: u32 = 0x0400_0002;
const TYPE_U32: u32 = 0x0100_0001;
const TYPE_STAT: u32 = 0x0200_0001;

impl Resolver for Fake {
    fn field_name(&self, token: u32) -> Result<String> {
        Ok(match token {
            FIELD_A => "A".to_owned(),
            FIELD_BLOB => "Blob".to_owned(),
            _ => return Err(IlError::new(0, "bad field")),
        })
    }

    fn type_name(&self, token: u32) -> Result<String> {
        Ok(match token {
            TYPE_U32 => "System.UInt32".to_owned(),
            TYPE_STAT => "STAT".to_owned(),
            _ => return Err(IlError::new(0, "bad type")),
        })
    }

    fn method(&self, token: u32) -> Result<MethodInfo> {
        let info = |owner: &str, name: &str, names: &[&str], codes: &[u8], this, void| MethodInfo {
            owner: owner.to_owned(),
            name: name.to_owned(),
            param_names: names.iter().map(|n| (*n).to_owned()).collect(),
            param_codes: codes.to_vec(),
            has_this: this,
            returns_void: void,
        };
        Ok(match token {
            CTOR_STAT => info(
                "STAT",
                ".ctor",
                &["priority", "count", "clear"],
                &[0x05, 0x06, 0x09],
                true,
                true,
            ),
            CTOR_DICT => info(
                "System.Collections.Generic.Dictionary`2",
                ".ctor",
                &[],
                &[],
                true,
                true,
            ),
            CALL_ADD => info(
                "System.Collections.Generic.Dictionary`2",
                "Add",
                &["a0", "a1"],
                &[0x08, 0x0E],
                true,
                true,
            ),
            CALL_INIT => info(
                "System.Runtime.CompilerServices.RuntimeHelpers",
                "InitializeArray",
                &["a", "b"],
                &[NONE, NONE],
                false,
                true,
            ),
            _ => return Err(IlError::new(0, "bad method")),
        })
    }

    fn user_string(&self, token: u32) -> Result<String> {
        Ok(format!("us{}", token & 0xFF))
    }

    fn field_data(&self, token: u32, length: usize) -> Result<Vec<u8>> {
        assert_eq!(token, FIELD_BLOB);
        assert_eq!(length, 12);
        let mut data = Vec::new();
        for value in [1u32, 0x8000_0001, u32::MAX] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        Ok(data)
    }
}

fn token(op: u8, token: u32) -> Vec<u8> {
    let mut bytes = vec![op];
    bytes.extend_from_slice(&token.to_le_bytes());
    bytes
}

#[test]
fn constructor_arguments_take_parameter_names_and_types() {
    let mut il = Vec::new();
    il.extend([0x1F, 0xFF]); // ldc.i4.s -1 for a byte parameter
    il.extend([0x20, 0xFE, 0xFF, 0xFF, 0xFF]); // ldc.i4 -2 for a short parameter
    il.extend([0x20, 0x3F, 0xFC, 0xFF, 0xFF]); // ldc.i4 -961 as raw uint bits
    il.extend(token(0x73, CTOR_STAT));
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    let Tree::Object { type_name, args } = &statics["A"] else {
        panic!("not an object: {:?}", statics["A"]);
    };
    assert_eq!(type_name, "STAT");
    assert_eq!(args[0], ("priority".to_owned(), Tree::Int(255)));
    assert_eq!(args[1], ("count".to_owned(), Tree::Int(-2)));
    assert_eq!(args[2], ("clear".to_owned(), Tree::Int(4_294_966_335)));
}

#[test]
fn arrays_default_to_zero_and_store_by_index() {
    let mut il = vec![0x19]; // length 3
    il.extend(token(0x8D, TYPE_U32));
    il.push(0x25);
    il.push(0x17); // index 1
    il.extend([0x20, 0xFF, 0xFF, 0xFF, 0xFF]); // -1 stored into uint[]
    il.push(0x9E);
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    assert_eq!(
        statics["A"],
        Tree::Array {
            element: "System.UInt32".to_owned(),
            items: vec![Tree::Int(0), Tree::Int(4_294_967_295), Tree::Int(0)],
        }
    );
}

#[test]
fn initialize_array_reads_field_rva_data() {
    let mut il = vec![0x19];
    il.extend(token(0x8D, TYPE_U32));
    il.push(0x25);
    il.extend(token(0xD0, FIELD_BLOB));
    il.extend(token(0x28, CALL_INIT));
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    let Tree::Array { items, .. } = &statics["A"] else {
        panic!("not an array");
    };
    assert_eq!(
        items,
        &[
            Tree::Int(1),
            Tree::Int(0x8000_0001),
            Tree::Int(4_294_967_295)
        ]
    );
}

#[test]
fn dictionary_adds_keep_order() {
    let mut il = token(0x73, CTOR_DICT);
    for (key, text) in [(5u8, 0x70), (2, 0x71)] {
        il.push(0x25);
        il.extend([0x1F, key]);
        il.extend(token(0x72, text));
        il.extend(token(0x6F, CALL_ADD));
    }
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    assert_eq!(
        statics["A"],
        Tree::Map(vec![
            (Tree::Int(5), Tree::Str("us112".to_owned())),
            (Tree::Int(2), Tree::Str("us113".to_owned())),
        ])
    );
}

#[test]
fn unmodeled_loads_are_marked_not_guessed() {
    let mut il = token(0x7E, FIELD_BLOB);
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    assert!(statics["A"].contains_unsupported());
}

#[test]
fn branches_are_rejected() {
    let error = evaluate(&[0x2B, 0x00], &Fake).unwrap_err();
    assert!(error.context.contains("unsupported opcode"));
}

#[test]
fn stack_underflow_is_an_error() {
    assert!(evaluate(&[0x26], &Fake).is_err());
}

// ---- synthetic assembly image ---------------------------------------------

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn pad_to(out: &mut Vec<u8>, length: usize) {
    assert!(out.len() <= length, "layout overflow at {}", out.len());
    out.resize(length, 0);
}

/// One section mapped at RVA 0, so RVAs equal file offsets.
fn build_image() -> Vec<u8> {
    let mut strings = vec![0u8];
    let mut add_string = |text: &str| -> u16 {
        let index = strings.len() as u16;
        strings.extend_from_slice(text.as_bytes());
        strings.push(0);
        index
    };
    let s_int32 = add_string("Int32");
    let s_system = add_string("System");
    let s_module = add_string("<Module>");
    let s_tables = add_string("Tables");
    let s_arr = add_string("Arr");
    let s_cctor = add_string(".cctor");
    while strings.len() % 4 != 0 {
        strings.push(0);
    }
    let blob = vec![0u8, 3, 0x06, 0x1D, 0x08, 3, 0x00, 0x00, 0x01, 0, 0, 0];
    let (b_field, b_method) = (1u16, 5u16);
    let user_strings = vec![0u8, 0, 0, 0];

    let mut tables = Vec::new();
    put_u32(&mut tables, 0);
    tables.extend([2, 0, 0, 1]);
    let valid: u64 = 1 | (1 << 1) | (1 << 2) | (1 << 4) | (1 << 6);
    tables.extend_from_slice(&valid.to_le_bytes());
    tables.extend_from_slice(&0u64.to_le_bytes());
    for rows in [1u32, 1, 2, 1, 1] {
        put_u32(&mut tables, rows);
    }
    for value in [0u16, s_module, 0, 0, 0] {
        put_u16(&mut tables, value);
    }
    for value in [0u16, s_int32, s_system] {
        put_u16(&mut tables, value);
    }
    for (name, methods) in [(s_module, 1u16), (s_tables, 1)] {
        put_u32(&mut tables, 0);
        for value in [name, 0, 0, 1, methods] {
            put_u16(&mut tables, value);
        }
    }
    for value in [0x10u16, s_arr, b_field] {
        put_u16(&mut tables, value);
    }
    put_u32(&mut tables, 0x180);
    for value in [0u16, 0x10, s_cctor, b_method, 1] {
        put_u16(&mut tables, value);
    }
    while tables.len() % 4 != 0 {
        tables.push(0);
    }

    let streams: [(&str, &[u8]); 4] = [
        ("#~", &tables),
        ("#Strings", &strings),
        ("#US", &user_strings),
        ("#Blob", &blob),
    ];
    let mut header = Vec::new();
    put_u32(&mut header, 0x424A_5342);
    put_u16(&mut header, 1);
    put_u16(&mut header, 1);
    put_u32(&mut header, 0);
    put_u32(&mut header, 12);
    header.extend_from_slice(b"v4.0.30319\0\0");
    put_u16(&mut header, 0);
    put_u16(&mut header, 4);
    let directory_size: usize = streams
        .iter()
        .map(|(name, _)| 8 + (name.len() + 1).div_ceil(4) * 4)
        .sum();
    let mut offset = header.len() + directory_size;
    let mut directory = Vec::new();
    for (name, data) in &streams {
        put_u32(&mut directory, offset as u32);
        put_u32(&mut directory, data.len() as u32);
        directory.extend_from_slice(name.as_bytes());
        directory.push(0);
        while directory.len() % 4 != 0 {
            directory.push(0);
        }
        offset += data.len();
    }
    assert_eq!(directory.len(), directory_size);
    let mut metadata = header;
    metadata.extend(directory);
    for (_, data) in &streams {
        metadata.extend_from_slice(data);
    }

    // IL: int[2] { 5, -3 } stored to static Arr.
    let mut il = vec![0x18];
    il.extend(token(0x8D, 0x0100_0001));
    il.extend([0x25, 0x16, 0x1B, 0x9E]);
    il.extend([0x25, 0x17, 0x1F, 0xFD, 0x9E]);
    il.extend(token(0x80, 0x0400_0001));
    il.push(0x2A);
    let mut body = vec![((il.len() as u8) << 2) | 2];
    body.extend(il);

    let mut image = vec![0u8; 0x40];
    image[..2].copy_from_slice(b"MZ");
    image[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    image.extend_from_slice(b"PE\0\0");
    put_u16(&mut image, 0x8664);
    put_u16(&mut image, 1);
    image.extend([0u8; 12]);
    put_u16(&mut image, 0xF0);
    put_u16(&mut image, 0x22);
    let optional = image.len();
    put_u16(&mut image, 0x20B);
    pad_to(&mut image, optional + 112 + 14 * 8);
    put_u32(&mut image, 0x200);
    put_u32(&mut image, 0x48);
    pad_to(&mut image, optional + 0xF0);
    let section_at = image.len();
    image.extend_from_slice(b".text\0\0\0");
    put_u32(&mut image, 0x2000);
    put_u32(&mut image, 0);
    put_u32(&mut image, 0x2000);
    put_u32(&mut image, 0);
    pad_to(&mut image, section_at + 40);
    pad_to(&mut image, 0x180);
    image.extend(body);
    pad_to(&mut image, 0x200);
    put_u32(&mut image, 0x48);
    put_u16(&mut image, 2);
    put_u16(&mut image, 5);
    put_u32(&mut image, 0x300);
    put_u32(&mut image, metadata.len() as u32);
    pad_to(&mut image, 0x300);
    image.extend(metadata);
    pad_to(&mut image, 0x2000);
    image
}

#[test]
fn parses_a_synthetic_assembly_static_initializer() {
    let image = build_image();
    let assembly = Assembly::parse(&image).unwrap();
    let statics = assembly.static_initializer("Tables").unwrap();
    assert_eq!(
        statics["Arr"],
        Tree::Array {
            element: "System.Int32".to_owned(),
            items: vec![Tree::Int(5), Tree::Int(-3)],
        }
    );
}

#[test]
fn reports_missing_types_and_non_assemblies() {
    let image = build_image();
    let assembly = Assembly::parse(&image).unwrap();
    assert!(assembly.static_initializer("Missing").is_err());
    assert!(Assembly::parse(&[0u8; 64]).is_err());
}

#[test]
fn ldsfld_of_an_earlier_store_shares_the_value() {
    let mut il = vec![0x1B]; // ldc.i4.5
    il.extend(token(0x80, FIELD_A));
    il.extend(token(0x7E, FIELD_A));
    il.extend(token(0x80, FIELD_BLOB));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    assert_eq!(statics["Blob"], Tree::Int(5));
}
