let nextSymId = 0;
let localFile, fileIndex;

function Symbol(name, loc)
{
  this.name = name;
  this.loc = loc;
  this.id = fileIndex + "-" + nextSymId++;
  this.uses = [];
}

Symbol.prototype = {
  use(loc) {
    this.uses.push(loc);
  },
};

function SymbolTable()
{
  this.table = new Map();
}

SymbolTable.prototype = {
  put(name, symbol) {
    this.table.set(name, symbol);
  },

  get(name) {
    return this.table.get(name);
  },
};

function locBefore(loc1, loc2) {
  return loc1.start.line < loc2.start.line ||
         (loc1.start.line == loc2.start.line && loc1.start.column < loc2.start.column);
}

function locstr(loc)
{
  if (loc.source === localFile)
    return `${loc.start.line}:${loc.start.column}`;
  else
    return `${loc.source}:${loc.start.line}:${loc.start.column}`;
}

function nameValid(name)
{
  return name.indexOf(" ") == -1 && name.indexOf("\n") == -1 && name.indexOf("\0") == -1;
}

function memberPropLoc(expr)
{
  let idLoc = expr.loc;
  idLoc.start.line = idLoc.end.line;
  idLoc.start.column = idLoc.end.column - expr.property.name.length;
  return idLoc;
}

let Analyzer = {
  symbols: new SymbolTable(),
  symbolTableStack: [],

  nameForThis: null,

  enter() {
    this.symbolTableStack.push(this.symbols);
    this.symbols = new SymbolTable();
  },

  exit() {
    let old = this.symbols;
    this.symbols = this.symbolTableStack.pop();
    return old;
  },

  isToplevel() {
    return this.symbolTableStack.length == 0;
  },

  scoped(f) {
    this.enter();
    f();
    this.exit();
  },

  program(prog) {
    for (let stmt of prog.body) {
      this.statement(stmt);
    }
  },

  defProp(name, loc, extra) {
    if (!nameValid(name)) {
      return;
    }
    if (extra) {
      print(`${locstr(loc)} def ${name} #${name} ${extra}`);
    } else {
      print(`${locstr(loc)} def ${name} #${name}`);
    }
  },

  useProp(name, loc, extra) {
    if (!nameValid(name)) {
      return;
    }
    if (extra) {
      print(`${locstr(loc)} use ${name} #${name} ${extra}`);
    } else {
      print(`${locstr(loc)} use ${name} #${name}`);
    }
  },

  assignProp(name, loc, extra) {
    if (!nameValid(name)) {
      return;
    }
    if (extra) {
      print(`${locstr(loc)} assign ${name} #${name} ${extra}`);
    } else {
      print(`${locstr(loc)} assign ${name} #${name}`);
    }
  },

  defVar(name, loc) {
    if (!nameValid(name)) {
      return;
    }
    if (this.isToplevel()) {
      this.defProp(name, loc);
      return;
    }
    let sym = new Symbol(name, loc);
    this.symbols.put(name, sym);
    print(`${locstr(loc)} def ${name} ${sym.id}`);
  },

  findSymbol(name) {
    let sym = this.symbols.get(name);
    if (!sym) {
      for (let i = this.symbolTableStack.length - 1; i >= 0; i--) {
        sym = this.symbolTableStack[i].get(name);
        if (sym) {
          break;
        }
      }
    }
    return sym;
  },

  useVar(name, loc) {
    if (!nameValid(name)) {
      return;
    }
    let sym = this.findSymbol(name);
    if (!sym) {
      this.useProp(name, loc);
    } else {
      print(`${locstr(loc)} use ${name} ${sym.id}`);
    }
  },

  assignVar(name, loc) {
    if (!nameValid(name)) {
      return;
    }
    let sym = this.findSymbol(name);
    if (!sym) {
      this.assignProp(name, loc);
    } else {
      print(`${locstr(loc)} assign ${name} ${sym.id}`);
    }
  },

  statement(stmt) {
    switch (stmt.type) {
    case "EmptyStatement":
    case "BreakStatement":
    case "ContinueStatement":
    case "DebuggerStatement":
      break;

    case "BlockStatement":
      this.scoped(() => {
        for (let stmt2 of stmt.body) {
          this.statement(stmt2);
        }
      });
      break;

    case "ExpressionStatement":
      this.expression(stmt.expression);
      break;

    case "IfStatement":
      this.expression(stmt.test);
      this.statement(stmt.consequent);
      this.maybeStatement(stmt.alternate);
      break;

    case "LabeledStatement":
      this.statement(stmt.body);
      break;

    case "WithStatement":
      this.expression(stmt.object);
      this.statement(stmt.body);
      break;

    case "SwitchStatement":
      this.expression(stmt.discriminant);
      for (let scase of stmt.cases) {
        this.switchCase(scase);
      }
      break;

    case "ReturnStatement":
      this.maybeExpression(stmt.argument);
      break;

    case "ThrowStatement":
      this.expression(stmt.argument);
      break;

    case "TryStatement":
      this.statement(stmt.block);
      for (let guarded of stmt.guardedHandlers) {
        this.catchClause(guarded);
      }
      if (stmt.handler) {
        this.catchClause(stmt.handler);
      }
      this.maybeStatement(stmt.finalizer);
      break;

    case "WhileStatement":
      this.expression(stmt.test);
      this.statement(stmt.body);
      break;

    case "DoWhileStatement":
      this.statement(stmt.body);
      this.expression(stmt.test);
      break;

    case "ForStatement":
      this.scoped(() => {
        if (stmt.init && stmt.init.type == "VariableDeclaration") {
          this.variableDeclaration(stmt.init);
        } else if (stmt.init) {
          this.expression(stmt.init);
        }
        this.maybeExpression(stmt.test);
        this.maybeExpression(stmt.update);
        this.statement(stmt.body);
      });
      break;

    case "ForInStatement":
    case "ForOfStatement":
      this.scoped(() => {
        if (stmt.left && stmt.left.type == "VariableDeclaration") {
          this.variableDeclaration(stmt.left);
        } else {
          this.expression(stmt.left);
        }
        this.expression(stmt.right);
        this.statement(stmt.body);
      });
      break;

    case "LetStatement":
      this.scoped(() => {
        for (let decl of stmt.head) {
          this.variableDeclarator(decl);
        }
        this.statement(stmt.body);
      });
      break;

    case "FunctionDeclaration":
      this.defVar(stmt.id.name, stmt.loc);
      this.scoped(() => {
        for (let i = 0; i < stmt.params.length; i++) {
          this.pattern(stmt.params[i]);
          this.maybeExpression(stmt.defaults[i]);
        }
        if (stmt.rest) {
          this.defVar(stmt.rest.name, stmt.rest.loc);
        }
        if (stmt.body.type == "BlockStatement") {
          this.statement(stmt.body);
        } else {
          this.expression(stmt.body);
        }
      });
      break;

    case "VariableDeclaration":
      this.variableDeclaration(stmt);
      break;

    default:
      throw "Unexpected statement: " + stmt.type;
      break;
    }
  },

  variableDeclaration(decl) {
    for (let d of decl.declarations) {
      this.variableDeclarator(d);
    }
  },

  variableDeclarator(decl) {
    this.pattern(decl.id);

    let oldNameForThis = this.nameForThis;
    if (decl.id.type == "Identifier" && decl.init) {
      if (decl.init.type == "ObjectExpression") {
        this.nameForThis = decl.id.name;
      } else {
        // Handle Object.freeze({...})
      }
    }
    this.maybeExpression(decl.init);
    this.nameForThis = oldNameForThis;
  },

  maybeStatement(stmt) {
    if (stmt) {
      this.statement(stmt);
    }
  },

  maybeExpression(expr) {
    if (expr) {
      this.expression(expr);
    }
  },

  switchCase(scase) {
    if (scase.test) {
      this.expression(scase.test);
    }
    for (let stmt of scase.consequent) {
      this.statement(stmt);
    }
  },

  catchClause(clause) {
    this.pattern(clause.param);
    if (clause.guard) {
      this.expression(clause.guard);
    }
    this.statement(clause.body);
  },

  expression(expr) {
    if (!expr) print(Error().stack);

    switch (expr.type) {
    case "Identifier":
      this.useVar(expr.name, expr.loc);
      break;

    case "Literal":
      break;

    case "TemplateLiteral":
      for (let elt of expr.elements) {
        this.expression(elt);
      }
      break;

    case "ThisExpression":
      // Do something eventually!
      break;

    case "ArrayExpression":
    case "ArrayPattern":
      for (let elt of expr.elements) {
        this.maybeExpression(elt);
      }
      break;

    case "ObjectExpression":
    case "ObjectPattern":
      for (let prop of expr.properties) {
        let name;

        if (prop.key) {
          let loc;
          if (prop.key.type == "Identifier") {
            name = prop.key.name;
            loc = prop.key.loc;
          } else if (prop.key.type == "Literal" && typeof(prop.key.value) == "string") {
            name = prop.key.value;
            loc = prop.key.loc;
            loc.start.column++;
          }
          let extra = null;
          if (this.nameForThis) {
            extra = `${this.nameForThis}#${name}`;
          }
          if (name) {
            this.defProp(name, prop.key.loc, extra);
          }
        }

        this.expression(prop.value);
      }
      break;

    case "FunctionExpression":
    case "ArrowFunctionExpression":
      this.scoped(() => {
        if (expr.type == "FunctionExpression" && expr.id) {
          this.defVar(expr.id.name, expr.loc);
        }
        for (let i = 0; i < expr.params.length; i++) {
          this.pattern(expr.params[i]);
          this.maybeExpression(expr.defaults[i]);
        }
        if (expr.rest) {
          this.defVar(expr.rest.name, expr.rest.loc);
        }
        if (expr.body.type == "BlockStatement") {
          this.statement(expr.body);
        } else {
          this.expression(expr.body);
        }
      });
      break;

    case "SequenceExpression":
      for (let elt of expr.expressions) {
        this.expression(elt);
      }
      break;

    case "UnaryExpression":
    case "UpdateExpression":
      this.expression(expr.argument);
      break;

    case "AssignmentExpression":
      if (expr.left.type == "Identifier") {
        this.assignVar(expr.left.name, expr.left.loc);
      } else if (expr.left.type == "MemberExpression" && !expr.left.computed) {
        this.expression(expr.left.object);

        let extra = null;
        if (expr.left.object.type == "ThisExpression" && this.nameForThis) {
          extra = `${this.nameForThis}#${expr.left.property.name}`;
        } else if (expr.left.object.type == "Identifier") {
          extra = `${expr.left.object.name}#${expr.left.property.name}`;
        }
        this.assignProp(expr.left.property.name, memberPropLoc(expr.left), extra);
      } else {
        this.expression(expr.left);
      }

      let oldNameForThis = this.nameForThis;
      if (expr.left.type == "MemberExpression" &&
          !expr.left.computed)
      {
        if (expr.left.property.name == "prototype" &&
            expr.left.object.type == "Identifier")
        {
          this.nameForThis = expr.left.object.name;
        }
        if (expr.left.object.type == "ThisExpression") {
          this.nameForThis = expr.left.property.name;
        }
      }
      this.expression(expr.right);
      this.nameForThis = oldNameForThis;
      break;

    case "BinaryExpression":
    case "LogicalExpression":
      this.expression(expr.left);
      this.expression(expr.right);
      break;

    case "ConditionalExpression":
      this.expression(expr.test);
      this.expression(expr.consequent);
      this.expression(expr.alternate);
      break;

    case "NewExpression":
    case "CallExpression":
      this.expression(expr.callee);
      for (let arg of expr.arguments) {
        this.expression(arg);
      }
      break;

    case "MemberExpression":
      this.expression(expr.object);
      if (expr.computed) {
        this.expression(expr.property);
      } else {
        let extra = null;
        if (expr.object.type == "ThisExpression" && this.nameForThis) {
          extra = `${this.nameForThis}#${expr.property.name}`;
        } else if (expr.object.type == "Identifier") {
          extra = `${expr.object.name}#${expr.property.name}`;
        }

        this.useProp(expr.property.name, memberPropLoc(expr), extra);
      }
      break;

    case "YieldExpression":
      this.maybeExpression(expr.argument);
      break;

    case "SpreadExpression":
      this.expression(expr.expression);
      break;

    case "ComprehensionExpression":
    case "GeneratorExpression":
      this.scoped(() => {
        let before = locBefore(expr.body.loc, expr.blocks[0].loc);
        if (before) {
          this.expression(expr.body);
        }
        for (let block of expr.blocks) {
          this.comprehensionBlock(block);
        }
        this.maybeExpression(expr.filter);
        if (!before) {
          this.expression(expr.body);
        }
      });
      break;

    default:
      print(Error().stack);
      throw `Invalid expression ${expr.type}: ${JSON.stringify(expr.loc)}`;
      break;
    }
  },

  comprehensionBlock(block) {
    switch (block.type) {
    case "ComprehensionBlock":
      this.pattern(block.left);
      this.expression(block.right);
      break;

    case "ComprehensionIf":
      this.expression(block.test);
      break;
    }
  },

  pattern(pat) {
    if (!pat) {
      print(Error().stack);
    }

    switch (pat.type) {
    case "Identifier":
      this.defVar(pat.name, pat.loc);
      break;

    case "ObjectPattern":
      for (let prop of pat.properties) {
        this.pattern(prop.value);
      }
      break;

    case "ArrayPattern":
      for (let e of pat.elements) {
        if (e) {
          this.pattern(e);
        }
      }
      break;

    case "SpreadExpression":
      this.pattern(pat.expression);
      break;

    default:
      throw `Unexpected pattern: ${pat.type} ${JSON.stringify(pat)}`;
      break;
    }
  },
};

function analyzeFile(filename)
{
  let text = snarf(filename);

  let substitution = false;
  let lines = text.split("\n");
  let preprocessedLines = [];
  let branches = [true];
  for (let i = 0; i < lines.length; i++) {
    let line = lines[i];
    if (substitution) {
      line = line.replace(/@(\w+)@/, "''");
    }
    let tline = line.trim();
    if (tline.startsWith("#ifdef") || tline.startsWith("#ifndef") || tline.startsWith("#if ")) {
      preprocessedLines.push("// " + tline);
      branches.push(branches[branches.length-1]);
    } else if (tline.startsWith("#else") ||
               tline.startsWith("#elif") ||
               tline.startsWith("#elifdef") ||
               tline.startsWith("#elifndef")) {
      preprocessedLines.push("// " + tline);
      branches.pop();
      branches.push(false);
    } else if (tline.startsWith("#endif")) {
      preprocessedLines.push("// " + tline);
      branches.pop();
    } else if (!branches[branches.length-1]) {
      preprocessedLines.push("// " + tline);
    } else if (tline.startsWith("#include")) {
      /*
      let match = tline.match(/#include "?([A-Za-z0-9_.-]+)"?/);
      if (!match) {
        throw new Error(`Invalid include directive: ${filename}:${i+1}`);
      }
      let incfile = match[1];
      preprocessedLines.push(`PREPROCESSOR_INCLUDE("${incfile}");`);
      */
      preprocessedLines.push("// " + tline);
    } else if (tline.startsWith("#filter substitution")) {
      preprocessedLines.push("// " + tline);
      substitution = true;
    } else if (tline.startsWith("#filter")) {
      preprocessedLines.push("// " + tline);
    } else if (tline.startsWith("#expand")) {
      preprocessedLines.push(line.substring(String("#expand ").length));
    } else if (tline.startsWith("#")) {
      preprocessedLines.push("// " + tline);
    } else {
      preprocessedLines.push(line);
    }
  }

  text = preprocessedLines.join("\n");

  let ast = Reflect.parse(text, {loc: true, source: filename, line: 1});
  Analyzer.program(ast);
}

fileIndex = scriptArgs[0];
localFile = scriptArgs[1];
analyzeFile(localFile);
