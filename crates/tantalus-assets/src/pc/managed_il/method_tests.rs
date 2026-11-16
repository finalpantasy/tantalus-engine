//! Hand-built IL for rank-2 arrays and method-body runs; no install needed.

use super::*;

struct Fake;

const CTOR_2D: u32 = 0x0A00_0010;
const SET_2D: u32 = 0x0A00_0011;
const CALL_LOG: u32 = 0x0A00_0012;
const CALL_INIT: u32 = 0x0A00_0014;
const CTOR_POINT: u32 = 0x0600_0020;
const FIELD_A: u32 = 0x0400_0001;
const FIELD_BLOB: u32 = 0x0400_0002;
const FIELD_STATIC: u32 = 0x0400_0003;
const TYPE_U32: u32 = 0x0100_0001;

impl Resolver for Fake {
    fn field_name(&self, token: u32) -> Result<String> {
        Ok(match token {
            FIELD_A => "A".to_owned(),
            FIELD_BLOB => "Blob".to_owned(),
            FIELD_STATIC => "Shared".to_owned(),
            _ => return Err(IlError::new(0, "bad field")),
        })
    }

    fn type_name(&self, token: u32) -> Result<String> {
        match token {
            TYPE_U32 => Ok("System.UInt32".to_owned()),
            _ => Err(IlError::new(0, "bad type")),
        }
    }

    fn method(&self, token: u32) -> Result<MethodInfo> {
        let info = |owner: &str, name: &str, codes: &[u8], this, void, array| MethodInfo {
            owner: owner.to_owned(),
            name: name.to_owned(),
            param_names: (0..codes.len()).map(|i| format!("p{i}")).collect(),
            param_codes: codes.to_vec(),
            has_this: this,
            returns_void: void,
            array,
        };
        let two_d = || {
            Some(ArrayCtor {
                element: "System.Byte".to_owned(),
                rank: 2,
                value_type: true,
            })
        };
        Ok(match token {
            CTOR_2D => info("<typespec>", ".ctor", &[0x08, 0x08], true, true, two_d()),
            SET_2D => info(
                "<typespec>",
                "Set",
                &[0x08, 0x08, 0x05],
                true,
                true,
                two_d(),
            ),
            CALL_LOG => info("Logger", "Log", &[0x08], false, true, None),
            CALL_INIT => info(
                "System.Runtime.CompilerServices.RuntimeHelpers",
                "InitializeArray",
                &[0xFF, 0xFF],
                false,
                true,
                None,
            ),
            CTOR_POINT => info("Point", ".ctor", &[0x08, 0x08], true, true, None),
            _ => return Err(IlError::new(0, "bad method")),
        })
    }

    fn user_string(&self, _token: u32) -> Result<String> {
        Ok(String::new())
    }

    fn field_data(&self, token: u32, length: usize) -> Result<Vec<u8>> {
        assert_eq!(token, FIELD_BLOB);
        assert_eq!(length, 6);
        Ok(vec![1, 2, 3, 4, 5, 6])
    }
}

fn token(op: u8, token: u32) -> Vec<u8> {
    let mut bytes = vec![op];
    bytes.extend_from_slice(&token.to_le_bytes());
    bytes
}

/// `new byte[rows, cols]` on the stack.
fn new_2d(rows: u8, cols: u8) -> Vec<u8> {
    let mut il = vec![0x1F, rows, 0x1F, cols];
    il.extend(token(0x73, CTOR_2D));
    il
}

fn set_2d(row: u8, col: u8, value: i16) -> Vec<u8> {
    let mut il = vec![0x25, 0x1F, row, 0x1F, col, 0x20];
    il.extend(i32::from(value).to_le_bytes());
    il.extend(token(0x28, SET_2D));
    il
}

fn row(values: &[i64]) -> Tree {
    Tree::Array {
        element: "System.Byte".to_owned(),
        items: values.iter().map(|v| Tree::Int(*v)).collect(),
    }
}

#[test]
fn rank_two_arrays_fill_by_set_and_default_to_zero() {
    let mut il = new_2d(2, 3);
    il.extend(set_2d(0, 1, 7));
    il.extend(set_2d(1, 2, 300)); // wraps to a byte
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    assert_eq!(
        statics["A"],
        Tree::Array {
            element: "System.Byte[,]".to_owned(),
            items: vec![row(&[0, 7, 0]), row(&[0, 0, 44])],
        }
    );
}

#[test]
fn rank_two_arrays_read_row_major_blobs() {
    let mut il = new_2d(2, 3);
    il.push(0x25);
    il.extend(token(0xD0, FIELD_BLOB));
    il.extend(token(0x28, CALL_INIT));
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let statics = evaluate(&il, &Fake).unwrap();
    assert_eq!(
        statics["A"],
        Tree::Array {
            element: "System.Byte[,]".to_owned(),
            items: vec![row(&[1, 2, 3]), row(&[4, 5, 6])],
        }
    );
}

#[test]
fn rank_two_set_out_of_range_is_an_error_in_static_mode_and_a_stop_in_method_mode() {
    let mut il = new_2d(2, 2);
    il.extend(set_2d(2, 0, 1));
    il.push(0x2A);
    let error = evaluate(&il, &Fake).unwrap_err();
    assert!(error.to_string().contains("out of range"), "{error}");
    let run = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    let stop = run.stop.expect("method run must stop");
    assert!(stop.reason.contains("out of range"));
}

#[test]
fn seeded_statics_feed_ldsfld() {
    let mut il = token(0x7E, FIELD_STATIC);
    il.extend(token(0x80, FIELD_A));
    il.push(0x2A);
    let unseeded = evaluate(&il, &Fake).unwrap();
    assert!(matches!(unseeded["A"], Tree::Unsupported(_)));
    let seeds = BTreeMap::from([("Shared".to_owned(), 255)]);
    let seeded = evaluate_seeded(&il, &Fake, &seeds).unwrap();
    assert_eq!(seeded["A"], Tree::Int(255));
}

#[test]
fn counted_loop_records_one_call_per_iteration() {
    let mut il = vec![0x16, 0x0A, 0x2B, 0x0A]; // i = 0; br cond
    il.push(0x06); // body: ldloc.0
    il.extend(token(0x28, CALL_LOG));
    il.extend([0x06, 0x17, 0x58, 0x0A]); // i = i + 1
    il.extend([0x06, 0x19, 0x32, 0xF2]); // cond: i < 3 -> body
    il.push(0x2A);
    let run = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    assert_eq!(run.stop, None);
    let args: Vec<_> = run.calls.iter().map(|c| c.args.clone()).collect();
    assert_eq!(
        args,
        vec![vec![Tree::Int(0)], vec![Tree::Int(1)], vec![Tree::Int(2)]]
    );
    assert_eq!(run.locals[&0], Tree::Int(3));
}

#[test]
fn step_limit_stops_an_endless_loop_with_a_reason() {
    let il = [0x2B, 0xFE]; // br.s to itself
    let run = run_method(
        &il,
        &Fake,
        &RunOptions {
            step_limit: 50,
            ..RunOptions::default()
        },
    )
    .unwrap();
    assert_eq!(run.steps, 50);
    assert_eq!(run.stop.unwrap().reason, "step limit reached");
}

#[test]
fn array_literal_survives_a_stop_at_a_non_constant_branch() {
    let mut il = vec![0x18];
    il.extend(token(0x8D, TYPE_U32));
    il.extend([0x25, 0x16, 0x1F, 0x05, 0x9E]); // arr[0] = 5
    il.push(0x0A);
    il.extend([0x02, 0x2D, 0x01, 0x2A, 0x2A]); // ldarg.0; brtrue.s
    let run = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    let stop = run.stop.unwrap();
    assert_eq!(stop.offset, il.len() - 4);
    assert_eq!(stop.reason, "branch on a value that is not constant");
    assert_eq!(
        run.locals[&0],
        Tree::Array {
            element: "System.UInt32".to_owned(),
            items: vec![Tree::Int(5), Tree::Int(0)],
        }
    );
}

#[test]
fn integer_arguments_drive_branches_and_return_a_constant() {
    // ldarg.1; ldc.i4.2; blt.s L; ldc.i4.s 99; ret; L: ldc.i4.s 7; ret
    let il = [0x03, 0x18, 0x32, 0x03, 0x1F, 0x63, 0x2A, 0x1F, 0x07, 0x2A];
    let with = |value| RunOptions {
        args: vec![None, Some(value)],
        ..RunOptions::default()
    };
    let low = run_method(&il, &Fake, &with(1)).unwrap();
    assert_eq!(low.returned, Some(Tree::Int(7)));
    let high = run_method(&il, &Fake, &with(5)).unwrap();
    assert_eq!(high.returned, Some(Tree::Int(99)));
    let unknown = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    assert_eq!(unknown.returned, None);
    assert!(unknown.stop.is_some());
}

#[test]
fn switch_picks_a_case_and_falls_through_out_of_range() {
    let mut il = vec![0x02, 0x45];
    il.extend(2u32.to_le_bytes());
    il.extend(3i32.to_le_bytes()); // case 0 -> offset 17
    il.extend(6i32.to_le_bytes()); // case 1 -> offset 20
    il.extend([0x1F, 0x09, 0x2A, 0x1F, 0x0A, 0x2A, 0x1F, 0x0B, 0x2A]);
    let result = |value| {
        let options = RunOptions {
            args: vec![Some(value)],
            ..RunOptions::default()
        };
        run_method(&il, &Fake, &options).unwrap().returned
    };
    assert_eq!(result(0), Some(Tree::Int(10)));
    assert_eq!(result(1), Some(Tree::Int(11)));
    assert_eq!(result(2), Some(Tree::Int(9)));
    assert_eq!(result(-1), Some(Tree::Int(9)));
}

#[test]
fn field_stores_on_untracked_objects_are_recorded_and_tracked_ones_stop() {
    let mut il = vec![0x02, 0x1F, 0x05];
    il.extend(token(0x7D, FIELD_A)); // arg0.A = 5
    il.extend([0x18, 0x1F, 0x03]);
    il.extend(token(0x73, CTOR_POINT));
    il.extend([0x17]);
    il.extend(token(0x7D, FIELD_A)); // tracked Point.A = 1
    il.push(0x2A);
    let run = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    assert_eq!(run.stores, vec![("A".to_owned(), Tree::Int(5))]);
    assert_eq!(run.stop.unwrap().reason, "stfld on a tracked object");
}

#[test]
fn element_loads_read_tracked_arrays_and_mark_untracked_ones() {
    let mut il = vec![0x18];
    il.extend(token(0x8D, TYPE_U32));
    il.extend([0x25, 0x17, 0x1F, 0x09, 0x9E, 0x0A]);
    il.extend([0x06, 0x17, 0x94, 0x0B]); // local1 = arr[1] (ldelem.i4)
    il.extend([0x02, 0x17, 0x94, 0x0C]); // local2 = arg0[1]
    il.push(0x2A);
    let run = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    assert_eq!(run.locals[&1], Tree::Int(9));
    assert!(matches!(run.locals[&2], Tree::Unsupported(_)));
}

#[test]
fn unsupported_opcodes_stop_a_method_run_and_fail_a_static_run() {
    let il = [0x16, 0x65, 0x2A]; // ldc.i4.0; neg
    let run = run_method(&il, &Fake, &RunOptions::default()).unwrap();
    assert_eq!(run.stop.unwrap().reason, "unsupported opcode 0x65");
    assert!(evaluate(&il, &Fake).is_err());
}

#[test]
fn instruction_decoder_measures_switch_and_prefixed_opcodes() {
    let mut il = vec![0x45];
    il.extend(2u32.to_le_bytes());
    il.extend([0u8; 8]);
    il.extend([0xFE, 0x01]); // ceq
    il.extend([0x20, 1, 0, 0, 0]);
    il.push(0x2A);
    let decoded = instructions(&il).unwrap();
    let shape: Vec<_> = decoded
        .iter()
        .map(|i| (i.offset, i.opcode, i.len))
        .collect();
    assert_eq!(
        shape,
        vec![(0, 0x45, 13), (13, 0xFE01, 2), (15, 0x20, 5), (20, 0x2A, 1)]
    );
    assert!(instructions(&[0x20, 1]).is_err());
}
