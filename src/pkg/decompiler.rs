use anyhow::{bail, Result};
use std::fmt::Write as FmtWrite;

#[derive(Debug, Clone)]
pub enum Constant {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
}

#[derive(Debug, Clone)]
pub struct Prototype {
    pub source: String,
    pub line_defined: u32,
    pub last_line_defined: u32,
    pub num_upvalues: u8,
    pub num_params: u8,
    pub is_vararg: u8,
    pub max_stack_size: u8,
    pub instructions: Vec<u32>,
    pub constants: Vec<Constant>,
    pub prototypes: Vec<Prototype>,
}

struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8> {
        if self.pos >= self.data.len() {
            bail!("Unexpected EOF reading u8");
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_u32(&mut self) -> Result<u32> {
        if self.pos + 4 > self.data.len() {
            bail!("Unexpected EOF reading u32");
        }
        let bytes: [u8; 4] = self.data[self.pos..self.pos + 4].try_into()?;
        self.pos += 4;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_f64(&mut self) -> Result<f64> {
        if self.pos + 8 > self.data.len() {
            bail!("Unexpected EOF reading f64");
        }
        let bytes: [u8; 8] = self.data[self.pos..self.pos + 8].try_into()?;
        self.pos += 8;
        Ok(f64::from_le_bytes(bytes))
    }

    fn read_string(&mut self) -> Result<String> {
        let size = self.read_u32()? as usize;
        if size == 0 {
            return Ok(String::new());
        }
        if self.pos + size > self.data.len() {
            bail!("Unexpected EOF reading string of length {}", size);
        }
        let str_bytes = &self.data[self.pos..self.pos + size - 1]; // strip trailing null
        self.pos += size;
        Ok(String::from_utf8_lossy(str_bytes).into_owned())
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        if self.pos + n > self.data.len() {
            bail!("Unexpected EOF skipping {} bytes", n);
        }
        self.pos += n;
        Ok(())
    }
}

pub fn parse_lua_51(bytes: &[u8]) -> Result<Prototype> {
    if bytes.len() < 12 {
        bail!("Bytecode too short");
    }
    if &bytes[0..4] != b"\x1bLua" {
        bail!("Not a Lua bytecode file");
    }
    if bytes[4] != 0x51 {
        bail!("Unsupported Lua version: 0x{:02x}, expected 0x51", bytes[4]);
    }

    let mut reader = ByteReader::new(bytes);
    reader.skip(12)?; // skip 12-byte header
    parse_prototype(&mut reader)
}

fn parse_prototype(r: &mut ByteReader) -> Result<Prototype> {
    let source = r.read_string()?;
    let line_defined = r.read_u32()?;
    let last_line_defined = r.read_u32()?;
    let num_upvalues = r.read_u8()?;
    let num_params = r.read_u8()?;
    let is_vararg = r.read_u8()?;
    let max_stack_size = r.read_u8()?;

    // Code
    let num_code = r.read_u32()? as usize;
    let mut instructions = Vec::with_capacity(num_code);
    for _ in 0..num_code {
        instructions.push(r.read_u32()?);
    }

    // Constants
    let num_constants = r.read_u32()? as usize;
    let mut constants = Vec::with_capacity(num_constants);
    for _ in 0..num_constants {
        let tag = r.read_u8()?;
        let c = match tag {
            0 => Constant::Nil,
            1 => Constant::Bool(r.read_u8()? != 0),
            3 => Constant::Number(r.read_f64()?),
            4 => Constant::String(r.read_string()?),
            _ => Constant::Nil,
        };
        constants.push(c);
    }

    // Inner Prototypes
    let num_prototypes = r.read_u32()? as usize;
    let mut prototypes = Vec::with_capacity(num_prototypes);
    for _ in 0..num_prototypes {
        prototypes.push(parse_prototype(r)?);
    }

    // Debug info: Line positions
    let num_lines = r.read_u32()? as usize;
    r.skip(num_lines * 4)?;

    // Debug info: Locals
    let num_locals = r.read_u32()? as usize;
    for _ in 0..num_locals {
        let _name = r.read_string()?;
        r.skip(8)?; // start_pc (4), end_pc (4)
    }

    // Debug info: Upvalues
    let num_upvalue_names = r.read_u32()? as usize;
    for _ in 0..num_upvalue_names {
        let _ = r.read_string()?;
    }

    Ok(Prototype {
        source,
        line_defined,
        last_line_defined,
        num_upvalues,
        num_params,
        is_vararg,
        max_stack_size,
        instructions,
        constants,
        prototypes,
    })
}

fn format_constant(c: &Constant) -> String {
    match c {
        Constant::Nil => "nil".to_string(),
        Constant::Bool(b) => b.to_string(),
        Constant::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        Constant::String(s) => {
            let mut out = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
            out
        }
    }
}

pub fn decompile_proto(proto: &Prototype, indent: usize) -> String {
    let mut out = String::new();
    let pad = "    ".repeat(indent);

    // Register expressions
    let max_reg = proto.max_stack_size as usize + 32;
    let mut regs: Vec<String> = (0..max_reg).map(|i| format!("r_{}", i)).collect();

    let get_rk = |val: usize, regs: &[String], consts: &[Constant]| -> String {
        if (val & 0x100) != 0 {
            let k_idx = val & 0xFF;
            if k_idx < consts.len() {
                format_constant(&consts[k_idx])
            } else {
                format!("K_{}", k_idx)
            }
        } else if val < regs.len() {
            regs[val].clone()
        } else {
            format!("r_{}", val)
        }
    };

    let get_k = |idx: usize, consts: &[Constant]| -> String {
        if idx < consts.len() {
            match &consts[idx] {
                Constant::String(s) => s.clone(),
                other => format_constant(other),
            }
        } else {
            format!("K_{}", idx)
        }
    };

    let len = proto.instructions.len();
    let mut pc = 0;

    while pc < len {
        let inst = proto.instructions[pc];
        let op = (inst & 0x3F) as usize;
        let a = ((inst >> 6) & 0xFF) as usize;
        let c = ((inst >> 14) & 0x1FF) as usize;
        let b = ((inst >> 23) & 0x1FF) as usize;
        let bx = ((inst >> 14) & 0x3FFFF) as usize;

        match op {
            0 => {
                // OP_MOVE
                if b < regs.len() && a < regs.len() {
                    regs[a] = regs[b].clone();
                }
            }
            1 => {
                // OP_LOADK
                if a < regs.len() {
                    if bx < proto.constants.len() {
                        regs[a] = format_constant(&proto.constants[bx]);
                    } else {
                        regs[a] = format!("K_{}", bx);
                    }
                }
            }
            2 => {
                // OP_LOADBOOL
                if a < regs.len() {
                    regs[a] = (b != 0).to_string();
                }
                if c != 0 {
                    pc += 1;
                }
            }
            3 => {
                // OP_LOADNIL
                for i in a..=b {
                    if i < regs.len() {
                        regs[i] = "nil".to_string();
                    }
                }
            }
            4 => {
                // OP_GETUPVAL
                if a < regs.len() {
                    regs[a] = format!("upval_{}", b);
                }
            }
            5 => {
                // OP_GETGLOBAL
                let gname = get_k(bx, &proto.constants);
                if a < regs.len() {
                    regs[a] = gname;
                }
            }
            6 => {
                // OP_GETTABLE: R(A) := R(B)[RK(C)]
                let tbl = if b < regs.len() { &regs[b] } else { "tbl" };
                let key = get_rk(c, &regs, &proto.constants);
                let access = if key.starts_with('"') && key.ends_with('"') && key.len() > 2 {
                    let inner = &key[1..key.len() - 1];
                    if inner.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                        format!("{}.{}", tbl, inner)
                    } else {
                        format!("{}[{}]", tbl, key)
                    }
                } else {
                    format!("{}[{}]", tbl, key)
                };
                if a < regs.len() {
                    regs[a] = access;
                }
            }
            7 => {
                // OP_SETGLOBAL: Gbl[Kst(Bx)] := R(A)
                let gname = get_k(bx, &proto.constants);
                let val = if a < regs.len() { &regs[a] } else { "nil" };
                let _ = writeln!(out, "{}{} = {}", pad, gname, val);
            }
            8 => {
                // OP_SETUPVAL
                let val = if a < regs.len() { &regs[a] } else { "nil" };
                let _ = writeln!(out, "{}upval_{} = {}", pad, b, val);
            }
            9 => {
                // OP_SETTABLE: R(A)[RK(B)] := RK(C)
                let tbl = if a < regs.len() { &regs[a] } else { "tbl" };
                let key = get_rk(b, &regs, &proto.constants);
                let val = get_rk(c, &regs, &proto.constants);
                if key.starts_with('"') && key.ends_with('"') && key.len() > 2 {
                    let inner = &key[1..key.len() - 1];
                    if inner.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                        let _ = writeln!(out, "{}{}.{} = {}", pad, tbl, inner, val);
                    } else {
                        let _ = writeln!(out, "{}{}[{}] = {}", pad, tbl, key, val);
                    }
                } else {
                    let _ = writeln!(out, "{}{}[{}] = {}", pad, tbl, key, val);
                }
            }
            10 => {
                // OP_NEWTABLE
                if a < regs.len() {
                    regs[a] = "{}".to_string();
                }
            }
            11 => {
                // OP_SELF: R(A+1) := R(B); R(A) := R(B)[RK(C)]
                let obj = if b < regs.len() { regs[b].clone() } else { "obj".to_string() };
                let method = get_rk(c, &regs, &proto.constants);
                let method_name = if method.starts_with('"') && method.ends_with('"') && method.len() > 2 {
                    method[1..method.len() - 1].to_string()
                } else {
                    method
                };
                if a + 1 < regs.len() {
                    regs[a + 1] = obj.clone();
                }
                if a < regs.len() {
                    regs[a] = format!("{}:{}", obj, method_name);
                }
            }
            12..=17 => {
                let op_str = match op {
                    12 => "+",
                    13 => "-",
                    14 => "*",
                    15 => "/",
                    16 => "%",
                    17 => "^",
                    _ => "+",
                };
                let left = get_rk(b, &regs, &proto.constants);
                let right = get_rk(c, &regs, &proto.constants);
                if a < regs.len() {
                    regs[a] = format!("({} {} {})", left, op_str, right);
                }
            }
            18 => {
                let v = if b < regs.len() { &regs[b] } else { "0" };
                if a < regs.len() {
                    regs[a] = format!("(-{})", v);
                }
            }
            19 => {
                let v = if b < regs.len() { &regs[b] } else { "false" };
                if a < regs.len() {
                    regs[a] = format!("(not {})", v);
                }
            }
            20 => {
                let v = if b < regs.len() { &regs[b] } else { "val" };
                if a < regs.len() {
                    regs[a] = format!("#{}", v);
                }
            }
            21 => {
                let mut parts = Vec::new();
                for i in b..=c {
                    if i < regs.len() {
                        parts.push(regs[i].clone());
                    }
                }
                if a < regs.len() {
                    regs[a] = parts.join(" .. ");
                }
            }
            28 => {
                // OP_CALL: R(A), ... ,R(A+C-2) := R(A)(R(A+1), ... ,R(A+B-1))
                let func = if a < regs.len() { regs[a].clone() } else { "fn".to_string() };
                let mut args = Vec::new();

                let is_method = func.contains(':');
                let arg_start = if is_method { a + 2 } else { a + 1 };
                let arg_end = if b > 0 { a + b } else { a + 1 };

                for i in arg_start..arg_end {
                    if i < regs.len() {
                        args.push(regs[i].clone());
                    }
                }

                let call_expr = format!("{}({})", func, args.join(", "));

                if c == 1 {
                    let _ = writeln!(out, "{}{}", pad, call_expr);
                } else {
                    if a < regs.len() {
                        regs[a] = call_expr;
                    }
                }
            }
            29 => {
                let func = if a < regs.len() { &regs[a] } else { "fn" };
                let _ = writeln!(out, "{}return {}()", pad, func);
            }
            30 => {
                // OP_RETURN
                if b == 1 {
                    if pc + 1 < len {
                        let _ = writeln!(out, "{}return", pad);
                    }
                } else if b > 1 {
                    let mut rets = Vec::new();
                    for i in a..(a + b - 1) {
                        if i < regs.len() {
                            rets.push(regs[i].clone());
                        }
                    }
                    let _ = writeln!(out, "{}return {}", pad, rets.join(", "));
                }
            }
            31 => {
                let _ = writeln!(out, "{}end", pad);
            }
            32 => {
                let init = if a < regs.len() { &regs[a] } else { "1" };
                let limit = if a + 1 < regs.len() { &regs[a + 1] } else { "10" };
                let _ = writeln!(out, "{}for i_{} = {}, {} do", pad, a + 3, init, limit);
            }
            34 => {
                let tbl = if a < regs.len() { &regs[a] } else { "tbl" };
                for i in 1..=b {
                    if a + i < regs.len() {
                        let _ = writeln!(out, "{}{}[{}] = {}", pad, tbl, i, regs[a + i]);
                    }
                }
            }
            36 => {
                // OP_CLOSURE: R(A) := closure(KPROTO[Bx])
                if bx < proto.prototypes.len() {
                    let inner = &proto.prototypes[bx];
                    let mut params = Vec::new();
                    for i in 0..inner.num_params {
                        params.push(format!("arg_{}", i));
                    }
                    if inner.is_vararg != 0 {
                        params.push("...".to_string());
                    }
                    let body = decompile_proto(inner, indent + 1);
                    let func_code = format!(
                        "function({})\n{}{}",
                        params.join(", "),
                        body,
                        pad
                    );
                    if a < regs.len() {
                        regs[a] = func_code;
                    }
                }
            }
            _ => {}
        }

        pc += 1;
    }

    out
}

pub fn decompile_lua_51(bytes: &[u8]) -> Result<String> {
    let proto = parse_lua_51(bytes)?;
    let mut code = decompile_proto(&proto, 0);
    if code.trim().is_empty() {
        code.push_str("-- [mw-pkg-tool] Empty or stub module\n");
    }
    Ok(code)
}
