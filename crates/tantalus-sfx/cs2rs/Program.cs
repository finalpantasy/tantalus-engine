// cs2rs: translates Memoria's C# rewrite of FF9SpecialEffectPlugin.dll into
// Rust over the flat memory model of tantalus-sfx. Every pointer becomes a
// u64 address in `Memory`; module globals are loads and stores at their
// original virtual addresses; C# integer semantics (promotion, unchecked
// wrapping, explicit casts) come from Roslyn's semantic model.
using System.Numerics;
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

static class P
{
    public static CSharpCompilation Comp;
    public static string Root;
    /// <summary>
    /// Places where Memoria's rewrite and the DLL disagree, fixed in the source
    /// before translation so the port follows the DLL (each is logged in
    /// audit/approved-differences.csv). Every patch must match exactly once.
    /// </summary>
    static readonly (string File, string From, string To)[] Patches =
    {
        // StartPsxTask's request: the DLL zeroes only its first 8 bytes and
        // reads the entry block's flags at +0x14, which task scripts write
        // (Thunder's task stores its update function there). The rewrite
        // zeroes and reads +12 instead, so the flags were always 0.
        ("Structs.g.cs",
         "[StructLayout(LayoutKind.Explicit, Size = 16)]\ninternal unsafe struct Stack16_StartPsxTask_request\n{\n",
         "[StructLayout(LayoutKind.Explicit, Size = 24)]\ninternal unsafe struct Stack16_StartPsxTask_request\n{\n    [FieldOffset(20)] public int field_20;\n"),
        ("u0005_sub_180048B10.cs", "        request.field_12 = 0;\n", ""),
        ("u0005_sub_180048B10.cs", "int flags = request.field_12;", "int flags = request.field_20;"),
        // The DLL's frame holds `args` above `request`: the second pointer
        // handed to the script entry must fall inside the PSX address window
        // the first (`request`) opened, as it does in the DLL (the stack
        // grows down, so `args` is allocated first). Otherwise the sword
        // skills (188-191, 221, 299, 401, 409-412) open one window too many.
        ("u0005_sub_180048B10.cs", "        Span<int> args = stackalloc int[2];\n", ""),
        ("u0005_sub_180048B10.cs", "        Stack16_StartPsxTask_request request = default;\n", "        Span<int> args = stackalloc int[2];\n        Stack16_StartPsxTask_request request = default;\n"),
    };
    static readonly HashSet<(string, string)> applied = new();
    static string Patch(string path, string text)
    {
        foreach (var (file, from, to) in Patches)
        {
            if (Path.GetFileName(path) != file) continue;
            int at = text.IndexOf(from, StringComparison.Ordinal);
            if (at < 0 || text.IndexOf(from, at + 1, StringComparison.Ordinal) >= 0)
                throw new Exception($"patch must match once: {file}: {from}");
            text = text.Remove(at, from.Length).Insert(at, to);
            applied.Add((file, from));
        }
        return text;
    }
    static void Main(string[] args)
    {
        Root = args[0];
        var output = args[1];
        var files = Directory.GetFiles(Root, "*.cs", SearchOption.AllDirectories)
            .Where(f => !f.Contains("\\obj\\") && !f.Contains("\\bin\\") && !f.Contains("/obj/") && !f.Contains("/bin/")).ToList();
        var opts = new CSharpParseOptions(LanguageVersion.Preview);
        var trees = files.Select(f => CSharpSyntaxTree.ParseText(Patch(f, File.ReadAllText(f).Replace("\r\n", "\n")), opts, f)).ToList();
        foreach (var (file, from, _) in Patches)
            if (!applied.Contains((file, from))) throw new Exception($"patch not applied: {file}: {from}");
        trees.Add(CSharpSyntaxTree.ParseText("global using System; global using System.IO; global using System.Collections.Generic; global using System.Linq; global using System.Threading; global using System.Threading.Tasks;", opts));
        var tpa = ((string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")).Split(Path.PathSeparator);
        Comp = CSharpCompilation.Create("x", trees, tpa.Select(p => MetadataReference.CreateFromFile(p)),
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, allowUnsafe: true));
        var rewritten = trees.Where(t => t.FilePath.Replace('\\', '/').Contains("/Rewritten/")).OrderBy(t => t.FilePath).ToList();
        var errors = Comp.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error && d.Location.SourceTree != null && rewritten.Contains(d.Location.SourceTree)).ToList();
        if (errors.Count > 0) { foreach (var e in errors.Take(5)) Console.Error.WriteLine(e); return; }
        var g = new Gen();
        g.CollectCodeAt(trees.First(t => t.FilePath.EndsWith("CodeAddresses.g.cs")));
        foreach (var t in rewritten) g.Collect(t);
        foreach (var t in rewritten) g.Emit(t);
        g.EmitTail();
        File.WriteAllText(output, g.Out.ToString().Replace("\r\n", "\n"));
        Console.WriteLine($"{g.Methods} methods, {g.Failures.Count} failures");
        foreach (var f in g.Failures.Take(60)) Console.WriteLine("  " + f);
    }
}

class Unsupported : Exception { public Unsupported(string m) : base(m) { } }

// A place: a Rust local holding a value, or memory at an address.
class Place
{
    public string Local;      // Rust variable holding the value
    public string Addr;       // Rust expression of the u64 address
    public ITypeSymbol Type;
    public bool IsMem => Local == null;
}

class Var { public string Name; public bool Mem; }

class MethodCtx
{
    public string RustName;
    public Dictionary<ISymbol, Var> Vars = new(SymbolEqualityComparer.Default);
    public HashSet<ISymbol> MemVars = new(SymbolEqualityComparer.Default);
    public List<(string brk, string cont)> Loops = new();
    public List<string> Breakables = new(); // innermost break target label
    public bool UsesStack;
    public int Temp;
    public SemanticModel Model;
    public ITypeSymbol ReturnType;
    public bool ReturnsRef;
    public List<string> Pre = new();
}

class Gen
{
    public StringBuilder Out = new();
    public int Methods;
    public List<string> Failures = new();
    readonly Dictionary<IMethodSymbol, string> names = new(SymbolEqualityComparer.Default);
    readonly Dictionary<IMethodSymbol, List<ISymbol>> captures = new(SymbolEqualityComparer.Default);
    readonly Dictionary<IPropertySymbol, string> props = new(SymbolEqualityComparer.Default);
    readonly Dictionary<ISymbol, string> staticFields = new(SymbolEqualityComparer.Default);
    readonly List<(string name, string type, string init)> fieldDecls = new();
    readonly Dictionary<uint, string> codeAt = new();
    readonly HashSet<string> used = new();
    int constId;
    MethodCtx M;
    SemanticModel Model => M.Model;

    // ---------- collection ----------
    public void CollectCodeAt(SyntaxTree tree)
    {
        foreach (var line in tree.GetText().Lines)
        {
            var s = line.ToString().Trim();
            // 0x1030 => (nint)(...)&CodeThunk_InitMapperTable, // InitMapperTable
            if (!s.StartsWith("0x") || !s.Contains("=>") || !s.Contains("CodeThunk_")) continue;
            var rva = Convert.ToUInt32(s[..s.IndexOf(' ')], 16);
            var at = s.IndexOf("CodeThunk_") + 10;
            var end = at;
            while (end < s.Length && (char.IsLetterOrDigit(s[end]) || s[end] == '_')) end++;
            codeAt[rva] = s[at..end];
        }
    }

    public void Collect(SyntaxTree tree)
    {
        var model = P.Comp.GetSemanticModel(tree);
        foreach (var m in tree.GetRoot().DescendantNodes().OfType<MethodDeclarationSyntax>())
        {
            var sym = model.GetDeclaredSymbol(m);
            var cls = sym.ContainingType.Name;
            var baseName = cls == "Native" ? sym.Name : $"{cls}__{sym.Name}";
            var name = baseName;
            int n = 1;
            while (used.Contains(name)) name = $"{baseName}__{++n}";
            used.Add(name);
            names[sym] = name;
            foreach (var lf in m.DescendantNodes().OfType<LocalFunctionStatementSyntax>())
            {
                var ls = (IMethodSymbol)model.GetDeclaredSymbol(lf);
                var ln = $"{name}__{ls.Name}";
                while (used.Contains(ln)) ln += "_";
                used.Add(ln);
                names[ls] = ln;
            }
            ComputeCaptures(model, m);
        }
        foreach (var p in tree.GetRoot().DescendantNodes().OfType<PropertyDeclarationSyntax>())
        {
            var sym = model.GetDeclaredSymbol(p);
            var cls = sym.ContainingType.Name;
            var name = $"prop__{(cls == "Native" ? "" : cls + "__")}{sym.Name}";
            props[sym] = name;
        }
        foreach (var f in tree.GetRoot().DescendantNodes().OfType<FieldDeclarationSyntax>())
        {
            foreach (var v in f.Declaration.Variables)
            {
                var sym = (IFieldSymbol)model.GetDeclaredSymbol(v);
                if (sym.IsConst) continue;
                var name = $"st_{sym.ContainingType.Name}_{sym.Name}";
                staticFields[sym] = name;
                fieldDecls.Add((name, RT(sym.Type), v.Initializer == null ? Zero(sym.Type) : "__INIT__" + v.Initializer.Value));
            }
        }
    }

    void ComputeCaptures(SemanticModel model, MethodDeclarationSyntax m)
    {
        var lfs = m.DescendantNodes().OfType<LocalFunctionStatementSyntax>().ToList();
        var direct = new Dictionary<IMethodSymbol, List<ISymbol>>(SymbolEqualityComparer.Default);
        var calls = new Dictionary<IMethodSymbol, List<IMethodSymbol>>(SymbolEqualityComparer.Default);
        foreach (var lf in lfs)
        {
            var ls = (IMethodSymbol)model.GetDeclaredSymbol(lf);
            var caps = new List<ISymbol>();
            var cl = new List<IMethodSymbol>();
            foreach (var id in lf.DescendantNodes().OfType<IdentifierNameSyntax>())
            {
                var s = model.GetSymbolInfo(id).Symbol;
                if (s is ILocalSymbol or IParameterSymbol)
                {
                    var owner = s.ContainingSymbol;
                    bool inside = false;
                    for (var c = owner; c != null; c = c.ContainingSymbol)
                        if (SymbolEqualityComparer.Default.Equals(c, ls)) { inside = true; break; }
                    if (!inside && !caps.Contains(s, SymbolEqualityComparer.Default)) caps.Add(s);
                }
                else if (s is IMethodSymbol ms && ms.MethodKind == MethodKind.LocalFunction && !SymbolEqualityComparer.Default.Equals(ms, ls))
                    cl.Add(ms);
            }
            direct[ls] = caps; calls[ls] = cl;
        }
        bool changed = true;
        while (changed)
        {
            changed = false;
            foreach (var (k, v) in direct)
                foreach (var c in calls[k])
                    if (direct.TryGetValue(c, out var cv))
                        foreach (var s in cv)
                        {
                            bool own = false;
                            for (var o = s.ContainingSymbol; o != null; o = o.ContainingSymbol)
                                if (SymbolEqualityComparer.Default.Equals(o, k)) own = true;
                            if (!own && !v.Contains(s, SymbolEqualityComparer.Default)) { v.Add(s); changed = true; }
                        }
        }
        foreach (var (k, v) in direct) captures[k] = v;
    }

    // ---------- types ----------
    public static ITypeSymbol Under(ITypeSymbol t) =>
        t is INamedTypeSymbol { TypeKind: TypeKind.Enum } e ? e.EnumUnderlyingType : t;

    public static bool IsSpan(ITypeSymbol t) => t is INamedTypeSymbol n && (n.Name == "Span" || n.Name == "ReadOnlySpan") && n.TypeArguments.Length == 1;
    public static ITypeSymbol SpanElem(ITypeSymbol t) => ((INamedTypeSymbol)t).TypeArguments[0];
    public static bool IsPtr(ITypeSymbol t) => t.TypeKind is TypeKind.Pointer or TypeKind.FunctionPointer;
    public static bool IsStruct(ITypeSymbol t)
    {
        t = Under(t);
        if (t.SpecialType != SpecialType.None) return false;
        if (IsSpan(t) || t.Name is "UInt128" or "Int128") return false;
        return t.TypeKind == TypeKind.Struct;
    }

    public static string RT(ITypeSymbol t)
    {
        t = Under(t);
        switch (t.SpecialType)
        {
            case SpecialType.System_Boolean: return "bool";
            case SpecialType.System_Char: return "u16";
            case SpecialType.System_SByte: return "i8";
            case SpecialType.System_Byte: return "u8";
            case SpecialType.System_Int16: return "i16";
            case SpecialType.System_UInt16: return "u16";
            case SpecialType.System_Int32: return "i32";
            case SpecialType.System_UInt32: return "u32";
            case SpecialType.System_Int64: return "i64";
            case SpecialType.System_UInt64: return "u64";
            case SpecialType.System_IntPtr: return "i64";
            case SpecialType.System_UIntPtr: return "u64";
            case SpecialType.System_Single: return "f32";
            case SpecialType.System_Double: return "f64";
            case SpecialType.System_String: return "String";
            case SpecialType.System_Void: return "()";
            case SpecialType.System_Object: return "String";
        }
        if (IsPtr(t)) return "u64";
        if (t.Name == "UInt128") return "u128";
        if (t.Name == "Int128") return "i128";
        if (IsSpan(t)) return "Sp";
        if (t.TypeKind == TypeKind.Struct) return "u64";
        throw new Unsupported($"type {t}");
    }

    public static bool IsInt(ITypeSymbol t) => RT(t) is "i8" or "u8" or "i16" or "u16" or "i32" or "u32" or "i64" or "u64" or "u128" or "i128";
    public static bool IsFloat(ITypeSymbol t) => RT(t) is "f32" or "f64";

    static readonly Dictionary<ITypeSymbol, (int size, int align, Dictionary<IFieldSymbol, int> offsets)> layouts = new(SymbolEqualityComparer.Default);

    public static int Size(ITypeSymbol t) => Layout(t).size;
    public static int Align(ITypeSymbol t) => Layout(t).align;

    public static (int size, int align, Dictionary<IFieldSymbol, int> offsets) Layout(ITypeSymbol t)
    {
        t = Under(t);
        switch (RT(t))
        {
            case "bool": case "i8": case "u8": return (1, 1, null);
            case "i16": case "u16": return (2, 2, null);
            case "i32": case "u32": case "f32": return (4, 4, null);
            case "f64": return (8, 8, null);
            case "u128": case "i128": return (16, 16, null);
            case "i64": if (t.SpecialType != SpecialType.None) return (8, 8, null); break;
            case "u64": if (!IsStruct(t)) return (8, 8, null); break;
        }
        if (layouts.TryGetValue(t, out var l)) return l;
        var nt = (INamedTypeSymbol)t;
        var offsets = new Dictionary<IFieldSymbol, int>(SymbolEqualityComparer.Default);
        var attrs = nt.GetAttributes();
        var inline = attrs.FirstOrDefault(a => a.AttributeClass?.Name == "InlineArrayAttribute");
        var fields = nt.GetMembers().OfType<IFieldSymbol>().Where(f => !f.IsStatic && !f.IsConst).ToList();
        if (inline != null)
        {
            var count = (int)inline.ConstructorArguments[0].Value;
            var el = fields[0];
            var (es, ea, _) = Layout(el.Type);
            offsets[el] = 0;
            return layouts[t] = (es * count, ea, offsets);
        }
        var sl = attrs.FirstOrDefault(a => a.AttributeClass?.Name == "StructLayoutAttribute");
        bool explicitLayout = sl != null && (int)sl.ConstructorArguments[0].Value == 2;
        int declared = 0, pack = 0;
        if (sl != null)
            foreach (var na in sl.NamedArguments)
            {
                if (na.Key == "Size") declared = (int)na.Value.Value;
                if (na.Key == "Pack") pack = (int)na.Value.Value;
            }
        int size = 0, align = 1;
        foreach (var f in fields)
        {
            int fs, fa;
            if (f.IsFixedSizeBuffer)
            {
                var el = ((IPointerTypeSymbol)f.Type).PointedAtType;
                fs = Size(el) * f.FixedSize; fa = Align(el);
            }
            else { (fs, fa, _) = Layout(f.Type); }
            if (pack > 0) fa = Math.Min(fa, pack);
            int off;
            if (explicitLayout)
            {
                var fo = f.GetAttributes().FirstOrDefault(a => a.AttributeClass?.Name == "FieldOffsetAttribute");
                off = fo == null ? 0 : (int)fo.ConstructorArguments[0].Value;
            }
            else off = (size + fa - 1) / fa * fa;
            offsets[f] = off;
            size = Math.Max(size, off + fs);
            align = Math.Max(align, fa);
        }
        size = (size + align - 1) / align * align;
        if (declared > size) size = declared;
        if (size == 0) size = 1;
        return layouts[t] = (size, align, offsets);
    }

    static int FieldOffset(IFieldSymbol f)
    {
        var l = Layout(f.ContainingType);
        return l.offsets[f.OriginalDefinition];
    }

    static string Zero(ITypeSymbol t) => RT(t) switch
    {
        "bool" => "false",
        "String" => "String::new()",
        "Sp" => "Sp(0, 0)",
        "()" => "()",
        var r => $"0{r}",
    };

    // ---------- literals ----------
    static string Lit(object v, ITypeSymbol t)
    {
        var rt = RT(t);
        if (v == null) return Zero(t);
        if (rt == "String") return $"String::from({Str(v.ToString())})";
        if (rt == "bool") return (v is bool b ? b : Convert.ToInt64(v) != 0) ? "true" : "false";
        if (rt == "f64") { double d = Convert.ToDouble(v); return $"f64::from_bits(0x{BitConverter.DoubleToInt64Bits(d):x}u64)"; }
        if (rt == "f32") { float f = Convert.ToSingle(v); return $"f32::from_bits(0x{BitConverter.SingleToInt32Bits(f):x}u32)"; }
        BigInteger x = v switch
        {
            char c => c,
            bool b => b ? 1 : 0,
            double d => new BigInteger(Math.Truncate(d)),
            float f => new BigInteger(Math.Truncate(f)),
            ulong u => u,
            _ => new BigInteger(Convert.ToInt64(v)),
        };
        int bits = rt switch { "i8" or "u8" => 8, "i16" or "u16" => 16, "i32" or "u32" => 32, "u128" or "i128" => 128, _ => 64 };
        bool signed = rt.StartsWith("i");
        var mod = BigInteger.One << bits;
        x %= mod; if (x < 0) x += mod;
        if (signed && x >= mod / 2) x -= mod;
        if (signed && x == -(mod / 2)) return $"{rt}::MIN";
        var s = x < 0 ? $"({x}{rt})" : (x > 9 ? $"0x{x.ToString("x").TrimStart('0').PadLeft(1, '0')}{rt}" : $"{x}{rt}");
        return s;
    }

    static string Str(string s)
    {
        var b = new StringBuilder("\"");
        foreach (var c in s)
        {
            if (c == '"' || c == '\\') b.Append('\\').Append(c);
            else if (c == '\n') b.Append("\\n");
            else if (c < 32 || c > 126) b.Append($"\\u{{{(int)c:x}}}");
            else b.Append(c);
        }
        return b.Append('"').ToString();
    }

    static string Ident(string n) => n switch
    {
        "self" or "Self" or "super" or "crate" => n + "_",
        "type" or "ref" or "match" or "loop" or "fn" or "move" or "mod" or "in" or "use" or "where" or "box" or "impl" or "mut" or "pub" or "static" or "trait" or "unsafe" or "dyn" or "async" or "await" or "yield" or "gen" or "final" or "macro" or "priv" or "become" or "virtual" or "abstract" or "override" or "typeof" or "unsized" or "let" or "struct" or "enum" or "extern" or "true" or "false" or "as" or "const" or "else" or "if" or "while" or "for" or "return" or "break" or "continue" => "r#" + n,
        "_" => "_unused",
        _ => n,
    };

    // ---------- emission ----------
    public void Emit(SyntaxTree tree)
    {
        var model = P.Comp.GetSemanticModel(tree);
        var file = Path.GetFileName(tree.FilePath);
        foreach (var m in tree.GetRoot().DescendantNodes().OfType<MethodDeclarationSyntax>())
        {
            var sym = model.GetDeclaredSymbol(m);
            EmitMethod(model, file, sym, m.ParameterList, (SyntaxNode)m.Body ?? m.ExpressionBody?.Expression, m);
            foreach (var lf in m.DescendantNodes().OfType<LocalFunctionStatementSyntax>())
                EmitMethod(model, file, (IMethodSymbol)model.GetDeclaredSymbol(lf), lf.ParameterList, (SyntaxNode)lf.Body ?? lf.ExpressionBody?.Expression, lf);
        }
        foreach (var p in tree.GetRoot().DescendantNodes().OfType<PropertyDeclarationSyntax>())
        {
            var sym = model.GetDeclaredSymbol(p);
            SyntaxNode body = p.ExpressionBody?.Expression;
            if (body == null)
            {
                var get = p.AccessorList?.Accessors.FirstOrDefault(a => a.Keyword.Text == "get");
                body = (SyntaxNode)get?.Body ?? get?.ExpressionBody?.Expression;
            }
            if (body == null) { Failures.Add($"{file}: property {sym.Name} without getter"); continue; }
            EmitBody(model, file, props[sym], sym.Type, sym.ReturnsByRef, new List<(string, IParameterSymbol)>(), new List<ISymbol>(), body, p, p.Initializer?.Value);
        }
    }

    void EmitMethod(SemanticModel model, string file, IMethodSymbol sym, ParameterListSyntax pl, SyntaxNode body, SyntaxNode decl)
    {
        if (body == null) { Failures.Add($"{file}: {sym.Name} has no body"); return; }
        var ps = sym.Parameters.Select(p => (Ident(p.Name), p)).ToList();
        var caps = sym.MethodKind == MethodKind.LocalFunction ? captures.GetValueOrDefault(sym) ?? new() : new List<ISymbol>();
        EmitBody(model, file, names[sym], sym.ReturnType, sym.ReturnsByRef, ps, caps, body, decl, null);
    }

    void EmitBody(SemanticModel model, string file, string name, ITypeSymbol ret, bool retRef, List<(string, IParameterSymbol)> ps, List<ISymbol> caps, SyntaxNode body, SyntaxNode decl, ExpressionSyntax collectionInit)
    {
        M = new MethodCtx { RustName = name, Model = model, ReturnType = ret, ReturnsRef = retRef };
        Methods++;
        var sb = new StringBuilder();
        try
        {
            // Variables that must live in memory.
            foreach (var n in body.DescendantNodesAndSelf())
            {
                ExpressionSyntax target = null;
                if (n is PrefixUnaryExpressionSyntax pu && pu.IsKind(SyntaxKind.AddressOfExpression)) target = pu.Operand;
                if (n is ArgumentSyntax a && !a.RefKindKeyword.IsKind(SyntaxKind.None)) target = a.Expression;
                if (n is RefExpressionSyntax re) target = re.Expression;
                if (target != null)
                {
                    var root = RootSymbol(model, target);
                    if (root != null) M.MemVars.Add(root);
                }
            }
            // Captured variables written inside local functions must be in memory.
            foreach (var lf in body.DescendantNodes().OfType<LocalFunctionStatementSyntax>())
                foreach (var n in lf.DescendantNodes())
                {
                    ExpressionSyntax t = n switch
                    {
                        AssignmentExpressionSyntax asg => asg.Left,
                        PrefixUnaryExpressionSyntax pu when pu.IsKind(SyntaxKind.PreIncrementExpression) || pu.IsKind(SyntaxKind.PreDecrementExpression) => pu.Operand,
                        PostfixUnaryExpressionSyntax po => po.Operand,
                        _ => null,
                    };
                    if (t is IdentifierNameSyntax id && model.GetSymbolInfo(id).Symbol is ILocalSymbol or IParameterSymbol)
                    {
                        var s = model.GetSymbolInfo(id).Symbol;
                        var lsym = model.GetDeclaredSymbol(lf);
                        bool own = false;
                        for (var o = s.ContainingSymbol; o != null; o = o.ContainingSymbol) if (SymbolEqualityComparer.Default.Equals(o, lsym)) own = true;
                        if (!own) M.MemVars.Add(s);
                    }
                }
            var sig = new List<string>();
            var prologue = new StringBuilder();
            foreach (var c in caps)
            {
                var t = c is ILocalSymbol l ? l.Type : ((IParameterSymbol)c).Type;
                var cname = Ident(c.Name);
                bool mem = OuterIsMem(c) || IsStruct(t);
                M.Vars[c] = new Var { Name = cname, Mem = mem };
                sig.Add($"mut {cname}: {(mem ? "u64" : RT(t))}");
            }
            foreach (var (pn, p) in ps)
            {
                bool byRef = p.RefKind != RefKind.None;
                if (byRef) { M.Vars[p] = new Var { Name = pn, Mem = true }; sig.Add($"mut {pn}: u64"); continue; }
                if (IsStruct(p.Type))
                {
                    M.UsesStack = true;
                    M.Vars[p] = new Var { Name = pn, Mem = true };
                    sig.Add($"{pn}_in: u64");
                    prologue.Append($"let {pn} = self.salloc({Size(p.Type)}); self.m.copy({pn}, {pn}_in, {Size(p.Type)});\n");
                    continue;
                }
                if (M.MemVars.Contains(p))
                {
                    M.UsesStack = true;
                    M.Vars[p] = new Var { Name = pn, Mem = true };
                    sig.Add($"{pn}_in: {RT(p.Type)}");
                    prologue.Append($"let {pn} = self.salloc({Size(p.Type)}); self.st::<{RT(p.Type)}>({pn}, {pn}_in);\n");
                    continue;
                }
                M.Vars[p] = new Var { Name = pn, Mem = false };
                sig.Add($"mut {pn}: {RT(p.Type)}");
            }
            var rt = retRef ? "u64" : RT(ret);
            string inner;
            if (collectionInit != null || body is ExpressionSyntax)
            {
                var e = (ExpressionSyntax)(collectionInit ?? body);
                string v;
                if (e is CollectionExpressionSyntax ce) v = ConstSpan(ce, ret);
                else v = retRef ? Address(e) : E(e);
                inner = Flush() + v + "\n";
            }
            else
            {
                inner = Block((BlockSyntax)body, "");
                if (rt != "()") inner += "        #[allow(unreachable_code)]\n        { unreachable!() }\n";
            }
            var file2 = file;
            sb.Append($"    // {file2}\n");
            var sigs = string.Join(", ", sig);
            if (M.UsesStack)
            {
                sb.Append($"    pub fn {name}(&mut self{(sigs.Length > 0 ? ", " : "")}{sigs}) -> {rt} {{\n        let __sp = self.sp;\n        let __r = self.{name}__body({string.Join(", ", sig.Select(s => s.Replace("mut ", "").Split(':')[0]))});\n        self.sp = __sp;\n        __r\n    }}\n");
                sb.Append($"    fn {name}__body(&mut self{(sigs.Length > 0 ? ", " : "")}{sigs}) -> {rt} {{\n        if self.tracing {{ self.trace(\"{name}\"); }}\n{prologue}{inner}    }}\n\n");
            }
            else sb.Append($"    pub fn {name}(&mut self{(sigs.Length > 0 ? ", " : "")}{sigs}) -> {rt} {{\n        if self.tracing {{ self.trace(\"{name}\"); }}\n{prologue}{inner}    }}\n\n");
            Out.Append(sb);
        }
        catch (Unsupported ex)
        {
            Failures.Add($"{file}: {name}: {ex.Message}");
            var sigs = new List<string>();
            foreach (var c in caps) sigs.Add($"_{Ident(c.Name)}: {(OuterIsMem(c) ? "u64" : RT(c is ILocalSymbol l ? l.Type : ((IParameterSymbol)c).Type))}");
            foreach (var (pn, p) in ps) sigs.Add($"_{pn}: {(p.RefKind != RefKind.None ? "u64" : SafeRT(p.Type))}");
            Out.Append($"    // {file}: not translated: {ex.Message.Replace("\n", " ")}\n    pub fn {name}(&mut self{(sigs.Count > 0 ? ", " : "")}{string.Join(", ", sigs)}) -> {(retRef ? "u64" : SafeRT(ret))} {{\n        unimplemented!({Str(name + ": " + ex.Message)})\n    }}\n\n");
        }
    }

    static string SafeRT(ITypeSymbol t) { try { return RT(t); } catch (Unsupported) { return "()"; } }

    readonly Dictionary<ISymbol, bool> outerMem = new(SymbolEqualityComparer.Default);
    bool OuterIsMem(ISymbol c)
    {
        // The outer method's analysis: the variable is in memory there when its
        // address is taken, it is a ref parameter, or a local function writes it.
        var decl = c.DeclaringSyntaxReferences.FirstOrDefault()?.GetSyntax();
        var method = decl?.AncestorsAndSelf().FirstOrDefault(a => a is MethodDeclarationSyntax);
        if (method == null) return false;
        if (outerMem.TryGetValue(c, out var r)) return r;
        if (c is IParameterSymbol { RefKind: not RefKind.None }) return outerMem[c] = true;
        var model = P.Comp.GetSemanticModel(method.SyntaxTree);
        bool mem = false;
        foreach (var n in method.DescendantNodes())
        {
            ExpressionSyntax target = null;
            if (n is PrefixUnaryExpressionSyntax pu && pu.IsKind(SyntaxKind.AddressOfExpression)) target = pu.Operand;
            if (n is ArgumentSyntax a && !a.RefKindKeyword.IsKind(SyntaxKind.None)) target = a.Expression;
            if (n is RefExpressionSyntax re) target = re.Expression;
            if (target != null && SymbolEqualityComparer.Default.Equals(RootSymbol(model, target), c)) mem = true;
        }
        foreach (var lf in method.DescendantNodes().OfType<LocalFunctionStatementSyntax>())
            foreach (var n in lf.DescendantNodes())
            {
                ExpressionSyntax t = n switch
                {
                    AssignmentExpressionSyntax asg => asg.Left,
                    PrefixUnaryExpressionSyntax pu when pu.IsKind(SyntaxKind.PreIncrementExpression) || pu.IsKind(SyntaxKind.PreDecrementExpression) => pu.Operand,
                    PostfixUnaryExpressionSyntax po => po.Operand,
                    _ => null,
                };
                if (t is IdentifierNameSyntax id && SymbolEqualityComparer.Default.Equals(model.GetSymbolInfo(id).Symbol, c)) mem = true;
            }
        var type = c is ILocalSymbol l ? l.Type : ((IParameterSymbol)c).Type;
        if (IsStruct(type)) mem = true;
        return outerMem[c] = mem;
    }

    static ISymbol RootSymbol(SemanticModel model, ExpressionSyntax e)
    {
        while (true)
        {
            switch (e)
            {
                case ParenthesizedExpressionSyntax p: e = p.Expression; continue;
                case MemberAccessExpressionSyntax ma when ma.IsKind(SyntaxKind.SimpleMemberAccessExpression):
                    var t = model.GetTypeInfo(ma.Expression).Type;
                    if (t != null && IsStruct(t)) { e = ma.Expression; continue; }
                    return null;
                case ElementAccessExpressionSyntax ea:
                    var at = model.GetTypeInfo(ea.Expression).Type;
                    if (at != null && IsStruct(at)) { e = ea.Expression; continue; }
                    return null;
                case IdentifierNameSyntax id:
                    var s = model.GetSymbolInfo(id).Symbol;
                    return s is ILocalSymbol or IParameterSymbol ? s : null;
                case DeclarationExpressionSyntax de:
                    if (de.Designation is SingleVariableDesignationSyntax sv) return model.GetDeclaredSymbol(sv);
                    return null;
                default: return null;
            }
        }
    }

    string Tmp() => $"__t{M.Temp++}";
    string Flush() { var s = string.Concat(M.Pre); M.Pre.Clear(); return s; }

    // ---------- statements ----------
    string Block(BlockSyntax b, string ind)
    {
        var sb = new StringBuilder();
        foreach (var s in b.Statements) sb.Append(Stmt(s, ind + "        "));
        return sb.ToString();
    }

    string Body(StatementSyntax s, string ind) => s is BlockSyntax b ? Block(b, ind) : Stmt(s, ind + "        ");

    string Stmt(StatementSyntax s, string ind)
    {
        string text = StmtInner(s, ind);
        var pre = Flush();
        return pre + text;
    }

    string StmtInner(StatementSyntax s, string ind)
    {
        switch (s)
        {
            case BlockSyntax b: return ind + "{\n" + Block(b, ind) + ind + "}\n";
            case EmptyStatementSyntax: return "";
            case LocalFunctionStatementSyntax: return "";
            case CheckedStatementSyntax c: return ind + "{\n" + Block(c.Block, ind) + ind + "}\n";
            case UnsafeStatementSyntax u: return ind + "{\n" + Block(u.Block, ind) + ind + "}\n";
            case ExpressionStatementSyntax es:
                if (IsTracer(es.Expression)) return "";
                return ind + ExprStmt(es.Expression) + ";\n";
            case LocalDeclarationStatementSyntax ld:
                {
                    var sb = new StringBuilder();
                    foreach (var v in ld.Declaration.Variables) sb.Append(ind + Declare(v, ld.Declaration.Type));
                    // Pre-statements belong before; Stmt wraps in a block which would
                    // scope the variable away, so splice them here.
                    var pre = Flush();
                    return pre.Length > 0 ? pre + sb : sb.ToString();
                }
            case ReturnStatementSyntax r:
                if (r.Expression == null) return ind + "return;\n";
                return ind + $"return {(M.ReturnsRef ? Address(r.Expression is RefExpressionSyntax re ? re.Expression : r.Expression) : E(r.Expression))};\n";
            case IfStatementSyntax i:
                {
                    var c = E(i.Condition);
                    var pre = Flush();
                    var t = Body(i.Statement, ind);
                    var sb = new StringBuilder(pre + ind + $"if ({c}) {{\n{t}{ind}}}");
                    if (i.Else != null)
                    {
                        sb.Append(" else {\n" + Body(i.Else.Statement, ind) + ind + "}");
                    }
                    return sb.Append('\n').ToString();
                }
            case WhileStatementSyntax w:
                {
                    var l = $"'l{M.Temp++}";
                    M.Loops.Add((l, l)); M.Breakables.Add(l);
                    var c = E(w.Condition); var pre = Flush();
                    var b = Body(w.Statement, ind);
                    M.Loops.RemoveAt(M.Loops.Count - 1); M.Breakables.RemoveAt(M.Breakables.Count - 1);
                    return ind + $"{l}: loop {{\n{pre}{ind}    if !({c}) {{ break; }}\n{b}{ind}}};\n";
                }
            case DoStatementSyntax d:
                {
                    var l = $"'l{M.Temp++}"; var cl = $"'c{M.Temp++}";
                    M.Loops.Add((l, cl)); M.Breakables.Add(l);
                    var b = Body(d.Statement, ind);
                    M.Loops.RemoveAt(M.Loops.Count - 1); M.Breakables.RemoveAt(M.Breakables.Count - 1);
                    var c = E(d.Condition); var pre = Flush();
                    return ind + $"{l}: loop {{\n{ind}    {cl}: {{\n{b}{ind}    }}\n{pre}{ind}    if !({c}) {{ break; }}\n{ind}}};\n";
                }
            case ForStatementSyntax f:
                {
                    var sb = new StringBuilder(ind + "{\n");
                    if (f.Declaration != null) foreach (var v in f.Declaration.Variables) { var dcl = Declare(v, f.Declaration.Type); sb.Append(Flush() + ind + dcl); }
                    foreach (var i in f.Initializers) { var x = ExprStmt(i); sb.Append(Flush() + ind + x + ";\n"); }
                    var l =$"'l{M.Temp++}"; var cl = $"'c{M.Temp++}";
                    var c = f.Condition == null ? "true" : E(f.Condition); var pre = Flush();
                    M.Loops.Add((l, cl)); M.Breakables.Add(l);
                    var b = Body(f.Statement, ind);
                    M.Loops.RemoveAt(M.Loops.Count - 1); M.Breakables.RemoveAt(M.Breakables.Count - 1);
                    var inc = new StringBuilder();
                    foreach (var i in f.Incrementors) { var x = ExprStmt(i); inc.Append(Flush() + ind + "    " + x + ";\n"); }
                    sb.Append(ind + $"{l}: loop {{\n{pre}{ind}    if !({c}) {{ break; }}\n{ind}    {cl}: {{\n{b}{ind}    }}\n{inc}{ind}}}\n");
                    return sb.Append(ind + "}\n").ToString();
                }
            case BreakStatementSyntax: return ind + $"break {M.Breakables[^1]};\n";
            case ContinueStatementSyntax:
                {
                    var (brk, cont) = M.Loops[^1];
                    return brk == cont ? ind + $"continue {brk};\n" : ind + $"break {cont};\n";
                }
            case SwitchStatementSyntax sw: return Switch(sw, ind);
            case ThrowStatementSyntax: return ind + "panic!(\"throw\");\n";
            case FixedStatementSyntax fx:
                {
                    var sb = new StringBuilder(ind + "{\n");
                    foreach (var v in fx.Declaration.Variables)
                    {
                        var sym = (ILocalSymbol)Model.GetDeclaredSymbol(v);
                        var init = v.Initializer.Value;
                        var it = Model.GetTypeInfo(init).Type;
                        string val;
                        if (init is LiteralExpressionSyntax lit && lit.IsKind(SyntaxKind.Utf8StringLiteralExpression)) val = Utf8(lit) + ".0";
                        else if (it != null && IsSpan(it)) val = E(init) + ".0";
                        else val = E(init);
                        var name = Ident(sym.Name);
                        M.Vars[sym] = new Var { Name = name, Mem = false };
                        sb.Append(Flush() + ind + $"let mut {name}: u64 = {val};\n");
                    }
                    sb.Append(Body(fx.Statement, ind));
                    return sb.Append(ind + "}\n").ToString();
                }
            default: throw new Unsupported($"statement {s.Kind()}");
        }
    }

    bool IsTracer(ExpressionSyntax e) => e is InvocationExpressionSyntax inv && inv.Expression.ToString().EndsWith("Tracer.Enter");

    string Declare(VariableDeclaratorSyntax v, TypeSyntax ts)
    {
        var sym = (ILocalSymbol)Model.GetDeclaredSymbol(v);
        var name = Ident(sym.Name);
        var t = sym.Type;
        if (sym.IsRef)
        {
            M.Vars[sym] = new Var { Name = name, Mem = true };
            var init = v.Initializer.Value is RefExpressionSyntax r ? r.Expression : v.Initializer.Value;
            return $"let mut {name}: u64 = {Address(init)};\n";
        }
        if (IsStruct(t))
        {
            M.UsesStack = true;
            M.Vars[sym] = new Var { Name = name, Mem = true };
            var s = $"let {name}: u64 = self.salloc({Size(t)});\n";
            if (v.Initializer != null && !IsDefault(v.Initializer.Value))
                s += $"{{ let __v = {E(v.Initializer.Value)}; self.m.copy({name}, __v, {Size(t)}); }}\n";
            return s;
        }
        if (M.MemVars.Contains(sym))
        {
            M.UsesStack = true;
            M.Vars[sym] = new Var { Name = name, Mem = true };
            var s = $"let {name}: u64 = self.salloc({Size(t)});\n";
            if (v.Initializer != null) s += $"{{ let __v = {Init(v.Initializer.Value, t)}; self.st::<{RT(t)}>({name}, __v); }}\n";
            return s;
        }
        M.Vars[sym] = new Var { Name = name, Mem = false };
        return $"let mut {name}: {RT(t)} = {(v.Initializer == null ? Zero(t) : Init(v.Initializer.Value, t))};\n";
    }

    string Init(ExpressionSyntax e, ITypeSymbol t)
    {
        if (e is StackAllocArrayCreationExpressionSyntax || e is ImplicitStackAllocArrayCreationExpressionSyntax) return StackAlloc(e, t);
        return E(e);
    }

    bool IsDefault(ExpressionSyntax e) => e is LiteralExpressionSyntax l && l.IsKind(SyntaxKind.DefaultLiteralExpression) || e is DefaultExpressionSyntax;

    string StackAlloc(ExpressionSyntax e, ITypeSymbol target)
    {
        M.UsesStack = true;
        ITypeSymbol elem; ExpressionSyntax count = null; InitializerExpressionSyntax init;
        if (e is StackAllocArrayCreationExpressionSyntax s)
        {
            var at = (ArrayTypeSyntax)s.Type;
            elem = Model.GetTypeInfo(at.ElementType).Type;
            var rank = at.RankSpecifiers[0].Sizes[0];
            if (rank is not OmittedArraySizeExpressionSyntax) count = rank;
            init = s.Initializer;
        }
        else
        {
            var i = (ImplicitStackAllocArrayCreationExpressionSyntax)e;
            init = i.Initializer;
            var tt = Model.GetTypeInfo(e).Type;
            elem = IsSpan(tt) ? SpanElem(tt) : ((IPointerTypeSymbol)tt).PointedAtType;
        }
        var n = count != null ? $"({E(count)} as i64)" : $"{init.Expressions.Count}i64";
        var size = Size(elem);
        var a = Tmp();
        M.Pre.Add($"let {a}: u64 = self.salloc(({n} * {size}) as usize);\n");
        if (init != null)
            for (int k = 0; k < init.Expressions.Count; k++)
                M.Pre.Add($"{{ let __v = {E(init.Expressions[k])}; self.st::<{RT(elem)}>({a} + {k * size}, __v); }}\n");
        var tt2 = target ?? Model.GetTypeInfo(e).ConvertedType;
        return IsSpan(tt2) ? $"Sp({a}, {n} as i32)" : a;
    }

    string Switch(SwitchStatementSyntax sw, string ind)
    {
        var l = $"'s{M.Temp++}";
        var v = Tmp();
        var gt = Model.GetTypeInfo(sw.Expression).Type;
        var sb = new StringBuilder(ind + $"{l}: {{\n");
        var val = E(sw.Expression);
        sb.Append(Flush() + ind + $"    let {v}: {RT(gt)} = {val};\n");
        M.Breakables.Add(l);
        SwitchSectionSyntax def = null;
        bool first = true;
        foreach (var sec in sw.Sections)
        {
            var conds = new List<string>();
            bool isDefault = false;
            foreach (var lab in sec.Labels)
            {
                switch (lab)
                {
                    case DefaultSwitchLabelSyntax: isDefault = true; break;
                    case CaseSwitchLabelSyntax c: conds.Add($"{v} == {ConstOf(c.Value, gt)}"); break;
                    case CasePatternSwitchLabelSyntax cp:
                        var pc = Pattern(cp.Pattern, v, gt);
                        if (cp.WhenClause != null) pc = $"({pc}) && ({E(cp.WhenClause.Condition)})";
                        conds.Add(pc); break;
                }
            }
            if (isDefault) { def = sec; continue; }
            var body = new StringBuilder();
            foreach (var st in sec.Statements) body.Append(Stmt(st, ind + "    "));
            sb.Append(ind + (first ? "    if " : "    else if ") + $"({string.Join(") || (", conds)}) {{\n{body}{ind}    }}\n");
            first = false;
        }
        if (def != null)
        {
            var body = new StringBuilder();
            foreach (var st in def.Statements) body.Append(Stmt(st, ind + "    "));
            sb.Append(ind + (first ? "    " : "    else ") + $"{{\n{body}{ind}    }}\n");
        }
        M.Breakables.RemoveAt(M.Breakables.Count - 1);
        return sb.Append(ind + "}\n").ToString();
    }

    string ConstOf(ExpressionSyntax e, ITypeSymbol t)
    {
        var c = Model.GetConstantValue(e);
        if (!c.HasValue) throw new Unsupported($"case {e}");
        return Lit(c.Value, t);
    }

    string Pattern(PatternSyntax p, string v, ITypeSymbol t)
    {
        switch (p)
        {
            case ConstantPatternSyntax c: return $"{v} == {ConstOf(c.Expression, t)}";
            case RelationalPatternSyntax r: return $"{v} {r.OperatorToken.Text} {ConstOf(r.Expression, t)}";
            case DiscardPatternSyntax: return "true";
            case BinaryPatternSyntax b: return $"({Pattern(b.Left, v, t)}) {(b.IsKind(SyntaxKind.OrPattern) ? "||" : "&&")} ({Pattern(b.Right, v, t)})";
            case UnaryPatternSyntax u: return $"!({Pattern(u.Pattern, v, t)})";
            case ParenthesizedPatternSyntax pp: return Pattern(pp.Pattern, v, t);
            default: throw new Unsupported($"pattern {p.Kind()}");
        }
    }

    // ---------- expressions ----------
    // An expression statement: assignments and increments without a value.
    string ExprStmt(ExpressionSyntax e)
    {
        switch (e)
        {
            case AssignmentExpressionSyntax a: return Assign(a, false);
            case PostfixUnaryExpressionSyntax po: return IncDec(po.Operand, po.IsKind(SyntaxKind.PostIncrementExpression), false, false);
            case PrefixUnaryExpressionSyntax pu when pu.IsKind(SyntaxKind.PreIncrementExpression) || pu.IsKind(SyntaxKind.PreDecrementExpression):
                return IncDec(pu.Operand, pu.IsKind(SyntaxKind.PreIncrementExpression), true, false);
            default:
                var x = E(e);
                var t = Model.GetTypeInfo(e).Type;
                return t == null || t.SpecialType == SpecialType.System_Void ? x : $"let _ = {x}";
        }
    }

    // The value of e after its implicit conversion.
    public string E(ExpressionSyntax e)
    {
        var ti = Model.GetTypeInfo(e);
        var to = ti.ConvertedType ?? ti.Type;
        var c = Model.GetConstantValue(e);
        if (c.HasValue && to != null && to.SpecialType == SpecialType.System_Object)
            return c.Value is string cs ? $"String::from({Str(cs)})" : $"format!(\"{{}}\", {Lit(c.Value, ti.Type)})";
        if (c.HasValue && to != null && !IsSpan(to))
        {
            if (c.Value is string str) return $"String::from({Str(str)})";
            if (c.Value == null && to.SpecialType == SpecialType.System_String) return "String::new()";
            return Lit(c.Value, to);
        }
        if (to != null && to.SpecialType == SpecialType.System_Object) return $"format!(\"{{:?}}\", {Raw(e)})";
        if (e is StackAllocArrayCreationExpressionSyntax || e is ImplicitStackAllocArrayCreationExpressionSyntax) return StackAlloc(e, to);
        var raw = Raw(e);
        if (ti.Type == null) return raw;
        return Conv(raw, ti.Type, to);
    }

    string Conv(string x, ITypeSymbol from, ITypeSymbol to)
    {
        if (to == null || SymbolEqualityComparer.Default.Equals(from, to)) return x;
        var f = RT(from); var t = RT(to);
        if (f == t) return x;
        if (t == "String") return $"format!(\"{{:?}}\", {x})";
        if (f == "Sp" && t == "u64") return $"({x}).0";
        if (f == "bool" || t == "bool") throw new Unsupported($"bool conversion {from} -> {to}");
        // A pointer cut to 32 bits or less: module addresses as the loaded
        // module has them (see `Plugin::low32`).
        if (IsPtr(from) && t is "i32" or "u32" or "i16" or "u16" or "i8" or "u8") return $"(({{ let __p: u64 = {x}; self.low32(__p) }}) as {t})";
        return $"(({x}) as {t})";
    }

    string Raw(ExpressionSyntax e)
    {
        switch (e)
        {
            case ParenthesizedExpressionSyntax p: return $"({E(p.Expression)})";
            case CheckedExpressionSyntax c: return E(c.Expression);
            case LiteralExpressionSyntax lit:
                if (lit.IsKind(SyntaxKind.Utf8StringLiteralExpression)) return Utf8(lit);
                if (lit.IsKind(SyntaxKind.NullLiteralExpression)) return "0u64";
                if (lit.IsKind(SyntaxKind.DefaultLiteralExpression)) return DefaultOf(Model.GetTypeInfo(e).ConvertedType);
                throw new Unsupported($"literal {lit}");
            case DefaultExpressionSyntax d: return DefaultOf(Model.GetTypeInfo(e).Type);
            case CastExpressionSyntax cast:
                {
                    var tt = Model.GetTypeInfo(cast).Type;
                    var innerCast = cast.Expression is ParenthesizedExpressionSyntax pe ? pe.Expression : cast.Expression;
                    if (innerCast is CastExpressionSyntax ic && Model.GetTypeInfo(ic.Expression).Type is { } it0 && IsPtr(it0) && RT(tt) is "i32" or "u32")
                        return $"(({{ let __p: u64 = {E(ic.Expression)}; self.low32(__p) }}) as {RT(tt)})";
                    var ot = Model.GetTypeInfo(cast.Expression).ConvertedType ?? Model.GetTypeInfo(cast.Expression).Type;
                    return Conv(E(cast.Expression), ot, tt);
                }
            case BinaryExpressionSyntax b: return Binary(b);
            case PrefixUnaryExpressionSyntax u: return Prefix(u);
            case PostfixUnaryExpressionSyntax po: return IncDec(po.Operand, po.IsKind(SyntaxKind.PostIncrementExpression), false, true);
            case AssignmentExpressionSyntax a: return Assign(a, true);
            case ConditionalExpressionSyntax ce:
                {
                    var c = E(ce.Condition);
                    var savedPre = M.Pre.ToList(); M.Pre.Clear();
                    var w = E(ce.WhenTrue); var wp = Flush();
                    var f = E(ce.WhenFalse); var fp = Flush();
                    M.Pre.AddRange(savedPre);
                    return $"(if ({c}) {{ {wp}{w} }} else {{ {fp}{f} }})";
                }
            case InvocationExpressionSyntax inv:
                if (Model.GetSymbolInfo(inv).Symbol is IMethodSymbol { ReturnsByRef: true } || Model.GetSymbolInfo(inv).Symbol is IMethodSymbol { Name: "GetReference" })
                    return Load(PlaceOf(inv));
                return Invoke(inv);
            case SizeOfExpressionSyntax so: return $"{Size(Model.GetTypeInfo(so.Type).Type)}i32";
            case ObjectCreationExpressionSyntax oc: return Create(oc);
            case ImplicitObjectCreationExpressionSyntax ioc: return Create(ioc);
            case SwitchExpressionSyntax sw:
                {
                    var v = Tmp();
                    var gt = Model.GetTypeInfo(sw.GoverningExpression).Type;
                    var val = E(sw.GoverningExpression);
                    var sb = new StringBuilder($"{{ let {v}: {RT(gt)} = {val}; ");
                    bool first = true;
                    var savedPre = M.Pre.ToList(); M.Pre.Clear();
                    foreach (var arm in sw.Arms)
                    {
                        var pc = Pattern(arm.Pattern, v, gt);
                        if (arm.WhenClause != null) pc = $"({pc}) && ({E(arm.WhenClause.Condition)})";
                        var ae = E(arm.Expression); var ap = Flush();
                        sb.Append($"{(first ? "" : "else ")}if ({pc}) {{ {ap}{ae} }} ");
                        first = false;
                    }
                    M.Pre.AddRange(savedPre);
                    return sb.Append("else { panic!(\"switch\") } }").ToString();
                }
            case InterpolatedStringExpressionSyntax: return "String::new()";
            case RefExpressionSyntax r: return E(r.Expression);
            case StackAllocArrayCreationExpressionSyntax or ImplicitStackAllocArrayCreationExpressionSyntax: return StackAlloc(e, null);
            case IsPatternExpressionSyntax ip:
                {
                    var v = Tmp();
                    var gt = Model.GetTypeInfo(ip.Expression).Type;
                    return $"{{ let {v}: {RT(gt)} = {E(ip.Expression)}; {Pattern(ip.Pattern, v, gt)} }}";
                }
        }
        var place = PlaceOf(e);
        if (place != null) return Load(place);
        throw new Unsupported($"expression {e.Kind()}: {Short(e)}");
    }

    static string Short(SyntaxNode n) { var s = n.ToString().Replace("\n", " "); return s.Length > 80 ? s[..80] : s; }

    string DefaultOf(ITypeSymbol t)
    {
        if (IsStruct(t)) { M.UsesStack = true; return $"self.salloc({Size(t)})"; }
        return Zero(t);
    }

    string Utf8(LiteralExpressionSyntax lit)
    {
        var s = lit.Token.ValueText;
        var bytes = Encoding.UTF8.GetBytes(s);
        return $"Sp(self.const_data({constId++}, &[{string.Join(", ", bytes.Append((byte)0))}]), {bytes.Length})";
    }

    string ConstSpan(CollectionExpressionSyntax ce, ITypeSymbol spanType)
    {
        var el = SpanElem(spanType);
        var bytes = new List<byte>();
        foreach (var x in ce.Elements)
        {
            var v = Model.GetConstantValue(((ExpressionElementSyntax)x).Expression);
            if (!v.HasValue) throw new Unsupported("collection element");
            var big = Convert.ToInt64(v.Value);
            for (int k = 0; k < Size(el); k++) bytes.Add((byte)(big >> (8 * k)));
        }
        return $"Sp(self.const_data({constId++}, &[{string.Join(", ", bytes)}]), {ce.Elements.Count})";
    }

    string Create(BaseObjectCreationExpressionSyntax oc)
    {
        var t = Model.GetTypeInfo(oc).Type;
        var args = oc.ArgumentList?.Arguments ?? default;
        if (IsSpan(t))
        {
            if (args.Count != 2) throw new Unsupported("span ctor");
            return $"Sp({E(args[0].Expression)}, {E(args[1].Expression)})";
        }
        if (IsStruct(t) && args.Count == 0) { M.UsesStack = true; return $"self.salloc({Size(t)})"; }
        throw new Unsupported($"new {t}");
    }

    string Load(Place p)
    {
        if (!p.IsMem) return RT(p.Type) == "String" ? $"{p.Local}.clone()" : p.Local;
        if (IsStruct(p.Type)) return p.Addr;
        if (IsSpan(p.Type)) throw new Unsupported("span in memory");
        if (p.Addr.Contains("self.")) return $"{{ let __a: u64 = {p.Addr}; self.ld::<{RT(p.Type)}>(__a) }}";
        return $"self.ld::<{RT(p.Type)}>({p.Addr})";
    }

    string Store(Place p, string v)
    {
        if (!p.IsMem) return $"{p.Local} = {v}";
        if (IsStruct(p.Type)) return $"{{ let __a = {p.Addr}; let __v = {v}; self.m.copy(__a, __v, {Size(p.Type)}); }}";
        return $"{{ let __a = {p.Addr}; let __v = {v}; self.st::<{RT(p.Type)}>(__a, __v); }}";
    }

    // Address of a place (for &x, ref x, Unsafe.AsPointer(ref x)).
    string Address(ExpressionSyntax e)
    {
        var p = PlaceOf(e) ?? throw new Unsupported($"address of {Short(e)}");
        if (!p.IsMem) throw new Unsupported($"address of register {Short(e)}");
        return p.Addr;
    }

    // Binds a place's address to a temp so it is evaluated once.
    Place Pin(Place p, List<string> lets)
    {
        if (!p.IsMem) return p;
        if (System.Text.RegularExpressions.Regex.IsMatch(p.Addr, @"^(0x[0-9a-f]+u64|[A-Za-z_][A-Za-z0-9_]*)$")) return p;
        var t = Tmp();
        lets.Add($"let {t}: u64 = {p.Addr};");
        return new Place { Addr = t, Type = p.Type };
    }

    Place PlaceOf(ExpressionSyntax e)
    {
        switch (e)
        {
            case ParenthesizedExpressionSyntax p: return PlaceOf(p.Expression);
            case IdentifierNameSyntax id:
                {
                    var s = Model.GetSymbolInfo(id).Symbol;
                    return SymbolPlace(s, null, e);
                }
            case MemberAccessExpressionSyntax ma:
                {
                    var s = Model.GetSymbolInfo(ma).Symbol;
                    if (s is IFieldSymbol f && !f.IsStatic)
                    {
                        var it = Model.GetTypeInfo(ma.Expression).Type;
                        string baseAddr;
                        if (ma.IsKind(SyntaxKind.PointerMemberAccessExpression)) baseAddr = E(ma.Expression);
                        else if (IsStruct(it)) baseAddr = Address(ma.Expression);
                        else throw new Unsupported($"field of {it}");
                        var off = FieldOffset(f);
                        var addr = off == 0 ? baseAddr : $"({baseAddr}).wrapping_add({off})";
                        if (f.IsFixedSizeBuffer) return new Place { Local = $"({addr})", Type = f.Type };
                        return new Place { Addr = addr, Type = f.Type };
                    }
                    if (s is IPropertySymbol { Name: "Length" } && Model.GetTypeInfo(ma.Expression).Type?.SpecialType == SpecialType.System_String)
                        return new Place { Local = $"(({E(ma.Expression)}).len() as i32)", Type = ((IPropertySymbol)s).Type };
                    if (s is IPropertySymbol { Name: "Length" } && IsSpan(Model.GetTypeInfo(ma.Expression).Type))
                        return new Place { Local = $"({E(ma.Expression)}).1", Type = s is IPropertySymbol ps ? ps.Type : null };
                    return SymbolPlace(s, ma, e);
                }
            case PrefixUnaryExpressionSyntax u when u.IsKind(SyntaxKind.PointerIndirectionExpression):
                {
                    var pt = (IPointerTypeSymbol)(Model.GetTypeInfo(u.Operand).ConvertedType ?? Model.GetTypeInfo(u.Operand).Type);
                    return new Place { Addr = E(u.Operand), Type = pt.PointedAtType };
                }
            case ElementAccessExpressionSyntax ea:
                {
                    var bt = Model.GetTypeInfo(ea.Expression).Type;
                    var idx = ea.ArgumentList.Arguments[0].Expression;
                    if (bt.SpecialType == SpecialType.System_String)
                        return new Place { Local = $"(({E(ea.Expression)}).as_bytes()[({E(idx)}) as usize] as u16)", Type = Model.GetTypeInfo(ea).Type };
                    if (bt is IPointerTypeSymbol pt)
                    {
                        var size = Size(pt.PointedAtType);
                        return new Place { Addr = $"pa({E(ea.Expression)}, ({E(idx)}) as i64, {size})", Type = pt.PointedAtType };
                    }
                    if (IsSpan(bt))
                    {
                        var el = SpanElem(bt);
                        return new Place { Addr = $"pa(({E(ea.Expression)}).0, ({E(idx)}) as i64, {Size(el)})", Type = el };
                    }
                    if (IsStruct(bt))
                    {
                        var l = Layout(bt);
                        var el = l.offsets.Keys.First().Type;
                        return new Place { Addr = $"pa({Address(ea.Expression)}, ({E(idx)}) as i64, {Size(el)})", Type = el };
                    }
                    throw new Unsupported($"element access on {bt}");
                }
            case InvocationExpressionSyntax inv:
                {
                    var ms = Model.GetSymbolInfo(inv).Symbol as IMethodSymbol;
                    if (ms != null && ms.Name == "GetReference" && ms.ContainingType.Name == "MemoryMarshal")
                    {
                        var st = Model.GetTypeInfo(inv.ArgumentList.Arguments[0].Expression).Type;
                        return new Place { Addr = $"({E(inv.ArgumentList.Arguments[0].Expression)}).0", Type = SpanElem(st) };
                    }
                    if (ms != null && ms.ReturnsByRef) return new Place { Addr = Invoke(inv), Type = ms.ReturnType };
                    if (ms != null && IsStruct(ms.ReturnType)) return new Place { Addr = Invoke(inv), Type = ms.ReturnType };
                    return null;
                }
        }
        return null;
    }

    Place SymbolPlace(ISymbol s, MemberAccessExpressionSyntax ma, ExpressionSyntax e)
    {
        switch (s)
        {
            case ILocalSymbol or IParameterSymbol:
                {
                    if (!M.Vars.TryGetValue(s, out var v)) throw new Unsupported($"unknown variable {s.Name}");
                    var t = s is ILocalSymbol l ? l.Type : ((IParameterSymbol)s).Type;
                    return v.Mem ? new Place { Addr = v.Name, Type = t } : new Place { Local = v.Name, Type = t };
                }
            case IPropertySymbol prop:
                {
                    var g = ModuleGlobal(prop);
                    if (g != null) return g;
                    if (props.TryGetValue(prop, out var pn))
                    {
                        if (prop.ReturnsByRef) return new Place { Addr = $"self.{pn}()", Type = prop.Type };
                        return new Place { Local = $"self.{pn}()", Type = prop.Type };
                    }
                    if (prop.Name.StartsWith("Code_") && prop.ContainingType.Name == "Native")
                    {
                        var target = prop.Name[5..];
                        var rva = codeAt.FirstOrDefault(kv => kv.Value == target).Key;
                        if (rva == 0) throw new Unsupported($"code address of {target}");
                        return new Place { Local = Lit(0x180000000UL + rva, prop.Type), Type = prop.Type };
                    }
                    if (prop.Name == "Length" && ma != null && IsSpan(Model.GetTypeInfo(ma.Expression).Type))
                        return new Place { Local = $"({E(ma.Expression)}).1", Type = prop.Type };
                    throw new Unsupported($"property {prop.ContainingType.Name}.{prop.Name}");
                }
            case IFieldSymbol f when f.IsStatic:
                {
                    if (staticFields.TryGetValue(f, out var fn)) return new Place { Local = $"self.statics.{fn}", Type = f.Type };
                    throw new Unsupported($"static field {f.ContainingType.Name}.{f.Name}");
                }
        }
        return null;
    }

    // ModuleData.g.cs: `ref *(T*)ModuleData.At(0xRVA)` (a ref accessor) or
    // `(T*)ModuleData.At(0xRVA)` (a pointer to the first byte).
    readonly Dictionary<IPropertySymbol, Place> globals = new(SymbolEqualityComparer.Default);
    Place ModuleGlobal(IPropertySymbol prop)
    {
        if (globals.TryGetValue(prop, out var g)) return g;
        var decl = prop.DeclaringSyntaxReferences.FirstOrDefault()?.GetSyntax() as PropertyDeclarationSyntax;
        if (decl == null || !decl.SyntaxTree.FilePath.EndsWith("ModuleData.g.cs")) return null;
        var body = decl.ExpressionBody.Expression.ToString();
        var at = body.IndexOf("ModuleData.At(");
        if (at < 0) return null;
        var hex = body[(at + 14)..body.IndexOf(')', at)];
        var addr = 0x180000000UL + Convert.ToUInt64(hex, 16);
        Place p = prop.ReturnsByRef ? new Place { Addr = $"0x{addr:x}u64", Type = prop.Type } : new Place { Local = $"0x{addr:x}u64", Type = prop.Type };
        return globals[prop] = p;
    }

    string Binary(BinaryExpressionSyntax b)
    {
        var k = b.Kind();
        if (k == SyntaxKind.LogicalAndExpression || k == SyntaxKind.LogicalOrExpression)
        {
            var l = E(b.Left);
            var saved = M.Pre.ToList(); M.Pre.Clear();
            var r = E(b.Right); var rp = Flush();
            M.Pre.AddRange(saved);
            if (rp.Length > 0) r = $"{{ {rp}{r} }}";
            return $"({l} {(k == SyntaxKind.LogicalAndExpression ? "&&" : "||")} {r})";
        }
        if (k == SyntaxKind.CoalesceExpression) throw new Unsupported("??");
        if (k == SyntaxKind.AsExpression || k == SyntaxKind.IsExpression) throw new Unsupported("as/is");
        var lt = Model.GetTypeInfo(b.Left).ConvertedType;
        var rt = Model.GetTypeInfo(b.Right).ConvertedType;
        var L = E(b.Left);
        var R = E(b.Right);
        return BinOp(k, L, lt, R, rt, Model.GetTypeInfo(b).Type);
    }

    static string BinOp(SyntaxKind k, string L, ITypeSymbol lt, string R, ITypeSymbol rt, ITypeSymbol result)
    {
        bool lp = IsPtr(lt), rp = IsPtr(rt);
        if (lp || rp)
        {
            if (k == SyntaxKind.AddExpression)
                return lp ? $"pa({L}, ({R}) as i64, {Size(((IPointerTypeSymbol)lt).PointedAtType)})" : $"pa({R}, ({L}) as i64, {Size(((IPointerTypeSymbol)rt).PointedAtType)})";
            if (k == SyntaxKind.SubtractExpression && lp && rp)
                return $"(({L}).wrapping_sub({R}) as i64 / {Size(((IPointerTypeSymbol)lt).PointedAtType)})";
            if (k == SyntaxKind.SubtractExpression && lp)
                return $"pa({L}, (({R}) as i64).wrapping_neg(), {Size(((IPointerTypeSymbol)lt).PointedAtType)})";
        }
        var op = k switch
        {
            SyntaxKind.AddExpression => "+", SyntaxKind.SubtractExpression => "-", SyntaxKind.MultiplyExpression => "*",
            SyntaxKind.DivideExpression => "/", SyntaxKind.ModuloExpression => "%",
            SyntaxKind.LeftShiftExpression => "<<", SyntaxKind.RightShiftExpression => ">>", SyntaxKind.UnsignedRightShiftExpression => ">>>",
            SyntaxKind.BitwiseAndExpression => "&", SyntaxKind.BitwiseOrExpression => "|", SyntaxKind.ExclusiveOrExpression => "^",
            SyntaxKind.EqualsExpression => "==", SyntaxKind.NotEqualsExpression => "!=",
            SyntaxKind.LessThanExpression => "<", SyntaxKind.LessThanOrEqualExpression => "<=",
            SyntaxKind.GreaterThanExpression => ">", SyntaxKind.GreaterThanOrEqualExpression => ">=",
            _ => throw new Unsupported($"binary {k}"),
        };
        if (RT(lt) == "String" || RT(result) == "String") return $"format!(\"{{}}{{}}\", {L}, {R})";
        bool isInt = IsInt(lt);
        switch (op)
        {
            case "+" when isInt: return $"({L}).wrapping_add({R})";
            case "-" when isInt: return $"({L}).wrapping_sub({R})";
            case "*" when isInt: return $"({L}).wrapping_mul({R})";
            case "/" when isInt: return $"({L}).wrapping_div({R})";
            case "%" when isInt: return $"({L}).wrapping_rem({R})";
            case "<<": return $"({L}).wrapping_shl(({R}) as u32)";
            case ">>": return $"({L}).wrapping_shr(({R}) as u32)";
            case ">>>": return $"((({L}) as u{RT(lt)[1..]}).wrapping_shr(({R}) as u32) as {RT(lt)})";
        }
        return $"({L} {op} {R})";
    }

    string Prefix(PrefixUnaryExpressionSyntax u)
    {
        switch (u.Kind())
        {
            case SyntaxKind.AddressOfExpression: return Address(u.Operand);
            case SyntaxKind.PointerIndirectionExpression: return Load(PlaceOf(u));
            case SyntaxKind.PreIncrementExpression: return IncDec(u.Operand, true, true, true);
            case SyntaxKind.PreDecrementExpression: return IncDec(u.Operand, false, true, true);
            case SyntaxKind.UnaryMinusExpression:
                {
                    var t = Model.GetTypeInfo(u.Operand).ConvertedType;
                    var x = E(u.Operand);
                    return IsInt(t) ? $"({x}).wrapping_neg()" : $"(-{x})";
                }
            case SyntaxKind.UnaryPlusExpression: return E(u.Operand);
            case SyntaxKind.BitwiseNotExpression: return $"(!{E(u.Operand)})";
            case SyntaxKind.LogicalNotExpression: return $"(!{E(u.Operand)})";
        }
        throw new Unsupported($"prefix {u.Kind()}");
    }

    string IncDec(ExpressionSyntax operand, bool inc, bool prefix, bool value)
    {
        var lets = new List<string>();
        var p = Pin(PlaceOf(operand) ?? throw new Unsupported("++ target"), lets);
        var t = p.Type;
        string next;
        var cur = Tmp();
        if (IsPtr(t)) next = $"pa({cur}, {(inc ? 1 : -1)}, {Size(((IPointerTypeSymbol)t).PointedAtType)})";
        else if (IsFloat(t)) next = $"{cur} {(inc ? "+" : "-")} 1.0";
        else next = $"{cur}.{(inc ? "wrapping_add" : "wrapping_sub")}(1)";
        var nv = Tmp();
        var body = $"let {cur}: {RT(t)} = {Load(p)}; let {nv}: {RT(t)} = {next}; {Store(p, nv)};";
        var pre = string.Join(" ", lets);
        if (!value) return $"{{ {pre} {body} }}";
        return $"{{ {pre} {body} {(prefix ? nv : cur)} }}";
    }

    string Assign(AssignmentExpressionSyntax a, bool value)
    {
        var lets = new List<string>();
        var k = a.Kind();
        // Discards: `_ = F();`
        if (a.Left is IdentifierNameSyntax { Identifier.Text: "_" } && Model.GetSymbolInfo(a.Left).Symbol is IDiscardSymbol or null)
            return value ? E(a.Right) : $"let _ = {E(a.Right)}";
        if (a.Left is TupleExpressionSyntax) throw new Unsupported("tuple assignment");
        var p = Pin(PlaceOf(a.Left) ?? throw new Unsupported($"assignment target {Short(a.Left)}"), lets);
        var t = p.Type;
        string v;
        if (k == SyntaxKind.SimpleAssignmentExpression)
        {
            if (a.Right is RefExpressionSyntax) throw new Unsupported("ref reassignment");
            if (IsStruct(t) && IsDefault(a.Right))
                return $"{{ {string.Join(" ", lets)} self.m.fill({p.Addr}, 0, {Size(t)}); }}";
            v = E(a.Right);
        }
        else
        {
            var opk = k switch
            {
                SyntaxKind.AddAssignmentExpression => SyntaxKind.AddExpression,
                SyntaxKind.SubtractAssignmentExpression => SyntaxKind.SubtractExpression,
                SyntaxKind.MultiplyAssignmentExpression => SyntaxKind.MultiplyExpression,
                SyntaxKind.DivideAssignmentExpression => SyntaxKind.DivideExpression,
                SyntaxKind.ModuloAssignmentExpression => SyntaxKind.ModuloExpression,
                SyntaxKind.AndAssignmentExpression => SyntaxKind.BitwiseAndExpression,
                SyntaxKind.OrAssignmentExpression => SyntaxKind.BitwiseOrExpression,
                SyntaxKind.ExclusiveOrAssignmentExpression => SyntaxKind.ExclusiveOrExpression,
                SyntaxKind.LeftShiftAssignmentExpression => SyntaxKind.LeftShiftExpression,
                SyntaxKind.RightShiftAssignmentExpression => SyntaxKind.RightShiftExpression,
                SyntaxKind.UnsignedRightShiftAssignmentExpression => SyntaxKind.UnsignedRightShiftExpression,
                _ => throw new Unsupported($"compound {k}"),
            };
            // The operator's operand type: the binary op symbol's first parameter.
            var opSym = Model.GetSymbolInfo(a).Symbol as IMethodSymbol;
            ITypeSymbol opT = opSym?.Parameters.Length == 2 ? opSym.Parameters[0].Type : t;
            ITypeSymbol rT = Model.GetTypeInfo(a.Right).ConvertedType;
            if (opSym != null && opSym.Parameters.Length == 2) rT = opSym.Parameters[1].Type;
            var r = E(a.Right);
            if (Model.GetTypeInfo(a.Right).ConvertedType is { } rct && rT != null && !SymbolEqualityComparer.Default.Equals(rct, rT)) r = Conv(r, rct, rT);
            var cur = Conv(Load(p), t, opT);
            var res = BinOp(opk, cur, opT, r, rT, opSym?.ReturnType ?? opT);
            v = Conv(res, opSym?.ReturnType ?? opT, t);
        }
        var nv = Tmp();
        var s = $"{{ {string.Join(" ", lets)} let {nv}: {RT(t)} = {v}; {Store(p, nv)}; {(value ? nv : "")} }}";
        return s;
    }

    // ---------- calls ----------
    string Invoke(InvocationExpressionSyntax inv)
    {
        var info = Model.GetSymbolInfo(inv);
        var ms = info.Symbol as IMethodSymbol;
        var callee = inv.Expression;
        var args = inv.ArgumentList.Arguments;
        if (ms == null)
        {
            var ct = Model.GetTypeInfo(callee).Type;
            if (ct is IFunctionPointerTypeSymbol fp)
            {
                var sig = fp.Signature;
                var fv = E(callee);
                var lets = new List<string>();
                var vals = new List<string>();
                for (int i = 0; i < args.Count; i++)
                {
                    var t = Tmp();
                    lets.Add($"let {t}: i64 = ({E(args[i].Expression)}) as i64;");
                    vals.Add(t);
                }
                lets.Insert(0, $"let __f: u64 = {fv};");
                var call = $"self.call_ptr(__f, &[{string.Join(", ", vals)}])";
                var ret = sig.ReturnsVoid ? $"{{ {string.Join(" ", lets)} {call}; }}" : $"{{ {string.Join(" ", lets)} ({call}) as {RT(sig.ReturnType)} }}";
                if (!sig.ReturnsVoid && RT(sig.ReturnType) == "bool") ret = $"{{ {string.Join(" ", lets)} ({call}) != 0 }}";
                return ret;
            }
            throw new Unsupported($"call {Short(inv)}");
        }
        var intr = Intrinsic(ms, inv);
        if (intr != null) return intr;
        if (!names.TryGetValue(ms.OriginalDefinition, out var name))
            throw new Unsupported($"call to {ms.ContainingType.Name}.{ms.Name}");
        var list = new List<string>();
        var letsB = new List<string>();
        if (ms.MethodKind == MethodKind.LocalFunction && captures.TryGetValue(ms.OriginalDefinition, out var caps))
            foreach (var c in caps)
            {
                if (!M.Vars.TryGetValue(c, out var v)) throw new Unsupported($"capture {c.Name}");
                list.Add(v.Name);
            }
        var byParam = new string[ms.Parameters.Length];
        for (int i = 0; i < args.Count; i++)
        {
            var a = args[i];
            int pi = a.NameColon != null ? ms.Parameters.IndexOf(ms.Parameters.First(p => p.Name == a.NameColon.Name.Identifier.Text)) : i;
            var p = ms.Parameters[pi];
            string val;
            if (p.RefKind != RefKind.None)
            {
                if (a.Expression is DeclarationExpressionSyntax de)
                {
                    var sym = (ILocalSymbol)Model.GetDeclaredSymbol((SingleVariableDesignationSyntax)de.Designation);
                    var nm = Ident(sym.Name);
                    M.UsesStack = true;
                    M.Vars[sym] = new Var { Name = nm, Mem = true };
                    M.Pre.Add($"let {nm}: u64 = self.salloc({Size(sym.Type)});\n");
                    val = nm;
                }
                else if (a.Expression is IdentifierNameSyntax { Identifier.Text: "_" })
                {
                    M.UsesStack = true;
                    val = $"self.salloc({Size(p.Type)})";
                }
                else val = Address(a.Expression);
            }
            else val = E(a.Expression);
            byParam[pi] = val;
        }
        for (int i = 0; i < byParam.Length; i++)
        {
            if (byParam[i] == null)
            {
                var p = ms.Parameters[i];
                if (!p.HasExplicitDefaultValue) throw new Unsupported($"missing argument {p.Name}");
                byParam[i] = Lit(p.ExplicitDefaultValue, p.Type);
            }
        }
        foreach (var v in byParam)
        {
            if (v.Contains("self."))
            {
                var t = Tmp();
                letsB.Add($"let {t} = {v};");
                list.Add(t);
            }
            else list.Add(v);
        }
        var callS = $"self.{name}({string.Join(", ", list)})";
        return letsB.Count == 0 ? callS : $"{{ {string.Join(" ", letsB)} {callS} }}";
    }

    string Arg(InvocationExpressionSyntax inv, int i) => E(inv.ArgumentList.Arguments[i].Expression);

    string Intrinsic(IMethodSymbol ms, InvocationExpressionSyntax inv)
    {
        var cls = ms.ContainingType?.Name;
        var n = ms.Name;
        var args = inv.ArgumentList.Arguments;
        string A(int i) => Arg(inv, i);
        switch (cls, n)
        {
            case ("Tracer", "Enter"): return "()";
            case ("DateTimeOffset", "ToUnixTimeSeconds"): return "self.unix_time()";
            case ("Math", "Sin"): return $"({A(0)}).sin()";
            case ("Math", "Cos"): return $"({A(0)}).cos()";
            case ("Math", "Sqrt"): return $"({A(0)}).sqrt()";
            case ("Math", "Atan2"): return $"({A(0)}).atan2({A(1)})";
            case ("Math", "Abs"): return IsInt(ms.Parameters[0].Type) ? $"({A(0)}).wrapping_abs()" : $"({A(0)}).abs()";
            case ("Math", "Min"): return $"({A(0)}).min({A(1)})";
            case ("Math", "Max"): return $"({A(0)}).max({A(1)})";
            case ("X86", "Cvttsd2si"): return $"cvttsd2si({A(0)})";
            case ("X86", "Cvttss2si"): return $"cvttsd2si(({A(0)}) as f64)";
            case ("BitConverter", "Int64BitsToDouble"): return $"f64::from_bits(({A(0)}) as u64)";
            case ("BitConverter", "SingleToInt32Bits"): return $"(({A(0)}).to_bits() as i32)";
            case ("BitConverter", "Int32BitsToSingle"): return $"f32::from_bits(({A(0)}) as u32)";
            case ("BitConverter", "DoubleToInt64Bits"): return $"(({A(0)}).to_bits() as i64)";
            case ("BitOperations", "LeadingZeroCount"): return $"(({A(0)}).leading_zeros() as i32)";
            case ("Buffer", "MemoryCopy"): return $"{{ let __s = {A(0)}; let __d = {A(1)}; let __n = ({A(3)}) as usize; self.m.copy(__d, __s, __n); }}";
            case ("Crt", "Memcpy"): return $"{{ let __d = {A(0)}; let __s = {A(1)}; let __n = ({A(2)}) as usize; self.m.copy(__d, __s, __n); __d }}";
            case ("Crt", "Memset"): return $"{{ let __d = {A(0)}; let __v = ({A(1)}) as u8; let __n = ({A(2)}) as usize; self.m.fill(__d, __v, __n); __d }}";
            case ("Unsafe", "AsPointer"): return Address(args[0].Expression);
            case ("Unsafe", "InitBlock"):
            case ("Unsafe", "InitBlockUnaligned"):
                {
                    var dst = args[0].RefKindKeyword.IsKind(SyntaxKind.None) ? A(0) : Address(args[0].Expression);
                    return $"{{ let __d = {dst}; let __v = ({A(1)}) as u8; let __n = ({A(2)}) as usize; self.m.fill(__d, __v, __n); }}";
                }
            case ("NativeMemory", "Alloc"):
            case ("NativeMemory", "AllocZeroed"): return $"{{ let __n = (({string.Join(") as usize * (", Enumerable.Range(0, args.Count).Select(A))}) as usize); self.m.alloc(__n) }}";
            case ("NativeMemory", "Free"): return $"{{ let _ = {A(0)}; }}";
            case ("Span", "Clear"):
                {
                    var st = Model.GetTypeInfo(((MemberAccessExpressionSyntax)inv.Expression).Expression).Type;
                    var sp = E(((MemberAccessExpressionSyntax)inv.Expression).Expression);
                    return $"{{ let __s = {sp}; self.m.fill(__s.0, 0, __s.1 as usize * {Size(SpanElem(st))}); }}";
                }
            case ("Span", "Fill"):
                {
                    var st = Model.GetTypeInfo(((MemberAccessExpressionSyntax)inv.Expression).Expression).Type;
                    var el = SpanElem(st);
                    var sp = E(((MemberAccessExpressionSyntax)inv.Expression).Expression);
                    return $"{{ let __s = {sp}; let __v = {A(0)}; for __i in 0..__s.1 as u64 {{ self.st::<{RT(el)}>(__s.0 + __i * {Size(el)}, __v); }} }}";
                }
            case ("Environment", "FailFast"): return $"{{ let __s = {A(0)}; self.fail_fast(__s) }}";
            case ("String", "Format"): case ("FormattableString", "Invariant"): case ("Int32", "ToString"): case ("UInt32", "ToString"): return "String::new()";
            case ("ModuleData", "At"): return $"(0x180000000u64 + ({A(0)}) as u64)";
            case ("Native", "CodeAt"): return $"(0x180000000u64 + ({A(0)}) as u64)";
            case ("IConsoleSink", "Write"): return $"{{ let __s = {A(0)}; self.console(__s) }}";
            case ("IRandomSource", "Seed"): return $"{{ let __s = ({A(0)}) as u32; self.crt_srand(__s) }}";
        }
        if (cls == "Native" && n.StartsWith("CodeThunk_")) throw new Unsupported($"thunk {n}");
        return null;
    }

    // ---------- tail ----------
    public void EmitTail()
    {
        var sb = new StringBuilder();
        sb.Append("    /// Calls the function the original's code address `address` points at\n    /// (the code table of `CodeAddresses.g.cs`), or the host callback.\n");
        sb.Append("    pub fn call_ptr(&mut self, address: u64, args: &[i64]) -> i64 {\n        let a = |i: usize| args.get(i).copied().unwrap_or(0);\n        if address == HOST_CALLBACK {\n            return self.host_callback(a(0) as i32, a(1) as i32, a(2) as i32, a(3) as i32, a(4) as i32, a(5) as u64) as i64;\n        }\n        match address.wrapping_sub(0x180000000) {\n");
        foreach (var (rva, target) in codeAt.OrderBy(kv => kv.Key))
        {
            var ms = names.Keys.FirstOrDefault(m => m.Name == target && m.MethodKind != MethodKind.LocalFunction && m.ContainingType.Name == "Native");
            if (ms == null) { sb.Append($"            // 0x{rva:x} {target}: not in Rewritten\n"); continue; }
            var argList = new List<string>();
            for (int i = 0; i < ms.Parameters.Length; i++)
            {
                var p = ms.Parameters[i];
                var rt = p.RefKind != RefKind.None ? "u64" : RT(p.Type);
                argList.Add(rt == "bool" ? $"a({i}) != 0" : $"a({i}) as {rt}");
            }
            var call = $"self.{names[ms]}({string.Join(", ", argList)})";
            var ret = ms.ReturnsVoid ? $"{{ {call}; 0 }}" : RT(ms.ReturnType) == "bool" ? $"i64::from({call})" : $"{call} as i64";
            sb.Append($"            0x{rva:x} => {ret},\n");
        }
        sb.Append("            _ => self.bad_call(address),\n        }\n    }\n");
        // Static fields of the rewrite's classes.
        var fs = new StringBuilder("/// Static fields of the rewrite's own classes (not module data).\n#[derive(Debug, Clone, Default)]\npub struct Statics {\n");
        foreach (var (name, type, init) in fieldDecls) fs.Append($"    pub {name}: {type},\n");
        fs.Append("}\n");
        var head = new StringBuilder();
        head.Append("// Generated by crates/tantalus-sfx/cs2rs from Memoria's FF9SpecialEffectPlugin rewrite\n// (MIT, Copyright (c) 2017 Albeoris). Do not edit; regenerate instead.\n#![allow(warnings, clippy::all, clippy::pedantic, clippy::nursery, non_snake_case, non_upper_case_globals, unused_parens, unused_braces, unused_labels)]\n\nuse super::rt::*;\n\n");
        head.Append(fs);
        head.Append("\nimpl Plugin {\n");
        Out.Insert(0, head.ToString());
        Out.Append(sb);
        Out.Append("}\n");
    }
}
