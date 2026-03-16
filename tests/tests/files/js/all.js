// Program
// (implicit)

// Identifier
USE_Identifier;

// Literal
10;

// FunctionDeclaration
function DEF_FUNC(NAME_FUNC = USE_FUNC) {
  return NAME_FUNC;
}
function* DEF_GEN(NAME_GEN = USE_GEN) {
  return NAME_GEN;
}
async function DEF_ASYNC(NAME_ASYNC = USE_ASYNC) {
  return NAME_ASYNC;
}
async function* DEF_ASYNC_GEN(NAME_ASYNC_GEN = USE_ASYNC_GEN) {
  return NAME_ASYNC_GEN;
}

// VariableDeclaration
// VariableDeclarator
var DEF_VAR = USE_VAR;
let DEF_LET = USE_LET;
const DEF_CONST = USE_CONST;

// SequenceExpression
USE_SEQ_1, USESEQ_2, USESEQ_3;

// ConditionalExpression
USE_COND_1 ? USE_COND_2 : USE_COND_3;

// UnaryExpression
delete USE_DEL.PROP_DEL;
-USE_NEG.PROP_NEG;
+USE_POS.PROP_POS;
!USE_NOT.PROP_NOT;
~USE_BITNOT.PROP_BITNOT;
typeof USE_TYPEOF.PROP_TYPEOF;
void USE_VOID.PROP;
async function DEF_ASYNC_AWAIT() {
  await USE_AWAIT.PROP_AWAIT;
}

// BinaryExpression
USE_EQ_1 == USE_EQ_2;
USE_NE_1 != USE_NE_2;
USE_STRIST_EQ_1 === USE_STRIST_EQ_2;
USE_STRIST_NE_1 !== USE_STRIST_NE_2;
USE_LT_1 < USE_LT_2;
USE_LE_1 <= USE_LE_2;
USE_GT_1 > USE_GT_2;
USE_GE_1 >= USE_GE_2;
USE_LSH_1 << USE_LSH_2;
USE_RSH_1 >> USE_RSH_2;
USE_URSH_1 >>> USE_URSH_2;
USE_ADD_1 + USE_ADD_2;
USE_SUB_1 - USE_SUB_2;
USE_MUL_1 * USE_MUL_2;
USE_DIV_1 / USE_DIV_2;
USE_MOD_1 % USE_MOD_2;
USE_POW_1 ** USE_POW_2;
USE_BITOR_1 | USE_BITOR_2;
USE_BITXOR_1 ^ USE_BITXOR_2;
USE_BITAND_1 & USE_BITAND_2;
USE_IN_1 in USE_IN_2;
USE_INSTANCEOF_1 instanceof USE_INSTANCEOF_2;
USE_COALESCE_1 ?? USE_COALESCE_2;

// AssignmentExpression
LHS_ASSIGN = USE_ASSIGN;
LHS_ASSIGN_ADD += USE_ASSIGN_ADD;
LHS_ASSIGN_SUB -= USE_ASSIGN_SUB;
LHS_ASSIGN_MUL *= USE_ASSIGN_MUL;
LHS_ASSIGN_DIV /= USE_ASSIGN_DIV;
LHS_ASSIGN_MOD %= USE_ASSIGN_MOD;
LHS_ASSIGN_POW **= USE_ASSIGN_POW;
LHS_ASSING_BIROR |= USE_ASSING_BITOR;
LHS_ASSING_BIRXOR ^= USE_ASSING_BIRXOR;
LHS_ASSING_BIRAND &= USE_ASSING_BIRAND;
LHS_ASSING_OR ||= USE_ASSING_OR;
LHS_ASSING_ND &&= USE_ASSING_AND;
LHS_ASSING_COALESCE ??= USE_ASSING_COALESCE;

// LogicalExpression
USE_OR_1 || USE_OR_2;
USE_COALECE_1 ?? USE_COALECE_2;
USE_AND_1 && USE_AND_2;

// UpdateExpression
LHS_INC_1++;
++LHS_INC_2;
LHS_DEC_1--;
--LHS_DEC_2;

// NewExpression
new REF_NEW_1(USE_NEW_1);
new REF_NEW_2.PROP_NEW(USE_NEW_2, ...USE_NEW_3);

// CallExpression
REF_CALL_1(USE_CALL_1);
REF_CALL_2.PROP_CALL(USE_CALL_2, ...USE_CALL_3);

// OptionalCallExpression
REF_OPT_CALL_1?.(USE_OPT_CALL_1);
REF_OPT_CALL_1.PROP_OPT_CALL?.(USE_OPT_CALL_2, ...USE_OPT_CALL_3);

// MemberExpression
REF_MEMBER_1.PROP_MEMBER;
REF_MEMBER_2[USE_MEMBER];

// OptionalMemberExpression
REF_OPT_MEMBER_1?.PROP_OPT_MEMBER;
REF_OPT_MEMBER_2?.[USE_OPT_MEMBER];

// FunctionExpression
(function DEF_FUNC_EXPR(NAME_FUNC_EXPR = USE_FUNC_EXPR) {
  return NAME_FUNC_EXPR;
});
(function(NAME_ANON_FUNC_EXPR = USE_ANON_FUNC_EXPR) {
  return NAME_ANON_FUNC_EXPR;
});
(function* DEF_GEN_EXPR(NAME_GEN_EXPR = USE_GEN_EXPR) {
  return NAME_GEN_EXPR;
});
(function* (NAME_ANON_GEN_EXPR = USE_ANON_GEN_EXPR) {
  return NAME_ANON_GEN_EXPR;
});
(async function DEF_ASYNC_EXPR(NAME_ASYNC_EXPR = USE_ASYNC_EXPR) {
  return NAME_ASYNC_EXPR;
});
(async function(NAME_ANON_ASYNC_EXPR = USE_ANON_ASYNC_EXPR) {
  return NAME_ANON_ASYNC_EXPR;
});
(async function* DEF_ASYNC_GEN_EXPR(NAME_ASYNC_GEN_EXPR = USE_ASYNC_GEN_EXPR) {
  return NAME_ASYNC_GEN_EXPR;
});
(async function* (NAME_ANON_ASYNC_GEN_EXPR = USE_ANON_ASYNC_GEN_EXPR) {
  return NAME_ANON_ASYNC_GEN_EXPR;
});

// ArrowFunctionExpression
(NAME_ARROW_1 = USE_ARROW) => {
  return NAME_ARROW;
};
NAME_ARROW_2 => {
  return NAME_ARROW_2;
};
async (NAME_ASYNC_ARROW_1 = USE_ASYNC_ARROW) => {
  return NAME_ASYNC_ARROW_1;
};

// ArrayExpression
[USE_ARRAY_1, , USE_ARRAY_2];

// DeleteOptionalExpression
delete USE_OPT_DEL?.PROP_OPT_DEL;

// OptionalExpression
USE_OPT_1?.PROP_OPT_1;
USE_OPT_2?.[USE_OPT_3];

// SpreadExpression
[...USE_SPREAD_1];
({ ...USE_SPREAD_1 });

// ObjectExpression
// Property
// ComputedName
({
  PROP_OBJ: USE_OBJ_1,
  [USE_OBJ_2]: USE_OBJ_2,
});
({
  get PROP_GETTER() { return USE_GETTER_1; },
  get [USE_GETTER_2]() { return USE_GETTER_3; },
});
({
  set PROP_SETTER(NAME_SETTER_1 = USE_SETTER_1) {
    USE_SETTER_2 = NAME_SETTER_1;
  },
  set [USE_SETTER_3](NAME_SETTER_2) {
    USE_SETTER_3 = NAME_SETTER_2;
  },
});
({
  PROP_METHOD(NAME_METHOD_1 = USE_METHOD_1) {
    USE_METHOD_2 = NAME_METHOD_1;
  },
  [USE_METHOD_3](NAME_METHOD_2 = USE_METHOD_4) {
    USE_METHOD_5 = NAME_METHOD_2;
  },
});

// PrototypeMutation
({
  __proto__: USE_PROTO,
});

// ThisExpression
this.PROP_THIS;
this[USE_THIS];

// YieldExpression
(function* () {
  yield USE_YIELD_1;
  yield* USE_YIELD_2;
});

// ClassStatement
class DEF_CLASS_1 {
  constructor() {
    USE_CLASS_1;
  }
};
class DEF_CLASS_2 extends USE_CLASS_2 {
};

// ClassMethod
(class {
  PROP_CLASS_METHOD(NAME_CLASS_METHOD_1 = USE_CLASS_METHOD_1) {
    return NAME_CLASS_METHOD_1;
  }
  [USE_CLASS_METHOD_2](NAME_CLASS_METHOD_2 = USE_CLASS_METHOD_3) {
    return NAME_CLASS_METHOD_2;
  }
});
(class {
  get PROP_CLASS_GETTER() { return USE_CLASS_GETTER_1; }
  get [USE_CLASS_GETTER_2]() { return USE_CLASS_GETTER_3; }
});
(class {
  set PROP_CLASS_SETTER(NAME_CLASS_SETTER_1 = USE_CLASS_SETTER_1) {
    USE_CLASS_SETTER_2 = NAME_CLASS_SETTER_1;
  }
  set [USE_CLASS_SETTER_3](NAME_CLASS_SETTER_2) {
    USE_CLASS_SETTER_4 = NAME_CLASS_SETTER_2;
  }
});

// ClassField
(class {
  DEF_FIELD_1;
  DEF_FIELD_2 = USE_FIELD_1;
});
(class {
  #DEF_PRIVATE_1;
  #DEF_PRIVATE_2 = USE_FIELD_2;
});

// StaticClassBlock
(class {
  static {
    USE_CLASS_STATIC;
  }
});

// ClassExpression
(class DEF_CLASS_EXPR {
});

// MetaProperty
(class {
  constructor() {
    new.target;
  }
});

// Super
(class extends USE_SUPER {
  PROP_SUPER_1() {
    super.PROP_SUPER_2;
  }
});

// CallImport
import("PATH");

// CallImportSource
// --

// EmptyStatement
;

// BlockStatement
{
  USE_BLOCK;
}

// ExpressionStatement
USE_EXPR;

// LabeledStatement
LABEL: USE_LABEL;

// IfStatement
if (USE_IF_1) {
  USE_IF_2;
} else {
  USE_IF_3;
}

// SwitchStatement
// SwitchCase
// CatchClause
switch (USE_SWITCH_1) {
  case USE_SWITCH_2:
    USE_SWITCH_3;
    break;
  default:
    USE_SWITCH_4;
    break;
}

// WhileStatement
while (USE_WHILE_1) {
  USE_WHILE_2;
}

// DoWhileStatement
do {
  USE_DO_1;
} while (USE_DO_2);

// ForStatement
for (USE_FOR_1; USE_FOR_2; USE_FOR_3) {
  USE_FOR_4;
}
for (var DEF_FOR_1 = 0; USE_FOR_5; USE_FOR_6) {
  USE_FOR_7;
}
for (let DEF_FOR_2 = 0; USE_FOR_8; USE_FOR_9) {
  USE_FOR_10;
}

// ForInStatement
for (LHS_FOR_IN_1 in USE_FOR_IN_1) {
  USE_FOR_IN_2;
}
for (var LHS_FOR_IN_2 in USE_FOR_IN_3) {
  USE_FOR_IN_4;
}
for (let LHS_FOR_IN_3 in USE_FOR_IN_5) {
  USE_FOR_IN_6;
}
for (const LHS_FOR_IN_4 in USE_FOR_IN_7) {
  USE_FOR_IN_8;
}

// ForOfStatement
for (LHS_FOR_OF_1 of USE_FOR_OF_1) {
  USE_FOR_OF_2;
}
for (var LHS_FOR_OF_2 of USE_FOR_OF_3) {
  USE_FOR_OF_4;
}
for (let LHS_FOR_OF_3 of USE_FOR_OF_5) {
  USE_FOR_OF_6;
}
for (const LHS_FOR_OF_4 of USE_FOR_OF_7) {
  USE_FOR_OF_8;
}
(async () => {
  for await (const LHS_FOR_OF_5 of USE_FOR_OF_9) {
    USE_FOR_OF_10;
  }
});

// BreakStatement
LABEL2: while (true) {
  break;
  break LABEL2;
}

// ContinueStatement
LABEL3: while (true) {
  continue;
  continue LABEL3;
}

// WithStatement
with (USE_WITH_1) {
  USE_WITH_2;
}

// ReturnStatement
(function() {
  return;
})
(function() {
  return USE_RETURN;
})

// TryStatement
try {
  USE_TRY_1;
} catch (DEF_CATCH_1) {
  USE_TRY_2;
}
try {
  USE_TRY_3;
} catch (DEF_CATCH_2) {
  USE_TRY_4;
} finally {
  USE_TRY_5;
}
try {
  USE_TRY_6;
} finally {
  USE_TRY_7;
}
try {
  USE_TRY_8;
} catch {
  USE_TRY_9;
}

// ThrowStatement
(function() {
  throw USE_THROW;
});

// DebuggerStatement
(function() {
  debugger;
});

// LetStatement
// --

// ArrayPattern
var [
  DEF_ARRAY_PAT_1,
  DEF_ARRAY_PAT_2 = USE_ARRAY_PAT_1,
  ...DEF_ARRAY_PAT_3
] = USE_ARRAY_PAT_2;
[
  LHS_ARRAY_PAT_1,
  LHS_ARRAY_PAT_2 = USE_ARRAY_PAT_3,
  ...LHS_ARRAY_PAT_3
] = USE_ARRAY_PAT_4;

// ObjectPattern
var {
  PROP_OBJ_PAT_1: DEF_OBJ_PAT_1,
  PROP_OBJ_PAT_2: DEF_OBJ_PAT_2 = USE_OBJ_PAT_1,
  DEF_OBJ_PAT_3 = USE_OBJ_PAT_2,
  ...DEF_OBJ_PAT_4
} = USE_OBJ_PAT_3;

({
  PROP_OBJ_PAT_3: LHS_OBJ_PAT_1,
  PROP_OBJ_PAT_4: LHS_OBJ_PAT_2 = USE_OBJ_PAT_4,
  LHS_OBJ_PAT_3 = USE_OBJ_PAT_5,
  ...LHS_OBJ_PAT_4
} = USE_OBJ_PAT_6);

// TemplateLiteral
`foo${USE_TMPL_1}${USE_TMPL_2}${USE_TMPL_3}`;

// TaggedTemplate
// CallSiteObject
USE_TAGGED_1`foo${USE_TAGGED_2}${USE_TAGGED_3}${USE_TAGGED_4}`;
