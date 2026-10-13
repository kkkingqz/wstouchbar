//! Add bounded test allocations to the real echo component without copying WIT.
use wasm_encoder::{Encode, Instruction, RawSection, RefType, Section, TableType};
use wasmparser::{Parser, Payload, TypeRef};
fn bytes(source: &[u8], range: std::ops::Range<u64>) -> &[u8] {
    &source[usize::try_from(range.start).unwrap()..usize::try_from(range.end).unwrap()]
}

pub fn component(initial: u64, grow: bool) -> Vec<u8> {
    let source = std::fs::read(touchbar_wasm_fixtures::ECHO).unwrap();
    let mut component = wasm_encoder::Component::new();
    let mut depth = 0;
    let mut patched = false;
    for payload in Parser::new(0).parse_all(&source) {
        let payload = payload.unwrap();
        match payload {
            Payload::Version { .. } => depth += 1,
            Payload::End(_) => depth -= 1,
            Payload::ModuleSection {
                unchecked_range, ..
            } if depth == 1 && !patched => {
                component.section(&RawSection {
                    id: 1,
                    data: &module(bytes(&source, unchecked_range), initial, grow),
                });
                patched = true;
            }
            _ if depth == 1 => {
                if let Some((id, range)) = payload.as_section() {
                    component.section(&RawSection {
                        id,
                        data: bytes(&source, range),
                    });
                }
            }
            _ => {}
        }
    }
    assert!(patched);
    component.finish()
}

fn module(source: &[u8], initial: u64, grow: bool) -> Vec<u8> {
    let mut imported = 0;
    let mut handle = None;
    let mut tables = 0;
    for payload in Parser::new(0).parse_all(source) {
        match payload.unwrap() {
            Payload::ImportSection(reader) => {
                imported = reader
                    .into_imports()
                    .map(|i| i.unwrap())
                    .filter(|i| matches!(i.ty, TypeRef::Func(_) | TypeRef::FuncExact(_)))
                    .count();
            }
            Payload::ExportSection(reader) => {
                handle = reader
                    .into_iter()
                    .map(|e| e.unwrap())
                    .find(|e| e.name.ends_with("#handle"))
                    .map(|e| e.index as usize);
            }
            Payload::TableSection(reader) => tables = reader.count(),
            _ => {}
        }
    }
    let handle = handle.expect("echo handle export") - imported;
    let mut module = wasm_encoder::Module::new();
    let mut code = Vec::new();
    let mut function = 0;
    for payload in Parser::new(0).parse_all(source) {
        let payload = payload.unwrap();
        match payload {
            Payload::TableSection(reader) => {
                let mut section = wasm_encoder::TableSection::new();
                wasm_encoder::reencode::Reencode::parse_table_section(
                    &mut wasm_encoder::reencode::RoundtripReencoder,
                    &mut section,
                    reader,
                )
                .unwrap();
                for _ in 0..2 {
                    section.table(TableType {
                        element_type: RefType::FUNCREF,
                        table64: false,
                        minimum: initial,
                        maximum: None,
                        shared: false,
                    });
                }
                module.section(&section);
            }
            Payload::CodeSectionStart { count, .. } => count.encode(&mut code),
            Payload::CodeSectionEntry(body) => {
                let bytes = bytes(source, body.range());
                let mut data = bytes.to_vec();
                if grow && function == handle {
                    let locals = usize::try_from(
                        body.get_operators_reader().unwrap().original_position()
                            - body.range().start,
                    )
                    .unwrap();
                    let mut prefix = Vec::new();
                    for table in tables..tables + 2 {
                        Instruction::RefNull(wasm_encoder::HeapType::FUNC).encode(&mut prefix);
                        Instruction::I32Const(700).encode(&mut prefix);
                        Instruction::TableGrow(table).encode(&mut prefix);
                        Instruction::I32Const(-1).encode(&mut prefix);
                        Instruction::I32Eq.encode(&mut prefix);
                        Instruction::If(wasm_encoder::BlockType::Empty).encode(&mut prefix);
                        Instruction::Unreachable.encode(&mut prefix);
                        Instruction::End.encode(&mut prefix);
                    }
                    data.splice(locals..locals, prefix);
                }
                data.len().encode(&mut code);
                code.extend(data);
                function += 1;
            }
            Payload::End(_) => {
                if !code.is_empty() {
                    module.section(&RawSection {
                        id: wasm_encoder::CodeSection::new().id(),
                        data: &code,
                    });
                }
            }
            _ => {
                if let Some((id, range)) = payload.as_section() {
                    // Code follows data-count but precedes data/custom sections.
                    if !code.is_empty() {
                        module.section(&RawSection {
                            id: 10,
                            data: &code,
                        });
                        code.clear();
                    }
                    module.section(&RawSection {
                        id,
                        data: bytes(source, range),
                    });
                }
            }
        }
    }
    module.finish()
}
