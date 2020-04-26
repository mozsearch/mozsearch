package org.mozilla.mozsearch;

import com.github.javaparser.ast.Node;
import com.github.javaparser.ast.NodeList;
import com.github.javaparser.ast.body.ClassOrInterfaceDeclaration;
import com.github.javaparser.ast.body.ConstructorDeclaration;
import com.github.javaparser.ast.body.EnumConstantDeclaration;
import com.github.javaparser.ast.body.EnumDeclaration;
import com.github.javaparser.ast.body.MethodDeclaration;
import com.github.javaparser.ast.body.Parameter;
import com.github.javaparser.ast.body.VariableDeclarator;
import com.github.javaparser.ast.expr.CastExpr;
import com.github.javaparser.ast.expr.FieldAccessExpr;
import com.github.javaparser.ast.expr.MethodCallExpr;
import com.github.javaparser.ast.expr.NameExpr;
import com.github.javaparser.ast.expr.ObjectCreationExpr;
import com.github.javaparser.ast.expr.SimpleName;
import com.github.javaparser.ast.stmt.CatchClause;
import com.github.javaparser.ast.type.ClassOrInterfaceType;
import com.github.javaparser.ast.type.ReferenceType;
import com.github.javaparser.ast.type.Type;
import com.github.javaparser.ast.visitor.VoidVisitorAdapter;
import com.github.javaparser.resolution.declarations.ResolvedConstructorDeclaration;
import com.github.javaparser.resolution.declarations.ResolvedEnumDeclaration;
import com.github.javaparser.resolution.declarations.ResolvedFieldDeclaration;
import com.github.javaparser.resolution.declarations.ResolvedMethodDeclaration;
import com.github.javaparser.resolution.declarations.ResolvedReferenceTypeDeclaration;
import com.github.javaparser.resolution.declarations.ResolvedTypeDeclaration;
import com.github.javaparser.resolution.declarations.ResolvedValueDeclaration;
import com.github.javaparser.resolution.types.ResolvedType;
import java.io.BufferedWriter;
import java.io.FileWriter;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;
import org.json.JSONObject;

public class MozSearchJSONOutputVisitor extends VoidVisitorAdapter<String> {
  private Path mOutputPath;
  private long mStart;
  private long mTimeout = 5 * 1000 * 60; // 5 min

  public MozSearchJSONOutputVisitor(final Path output) {
    mOutputPath = output;
    if (Files.exists(output)) {
      try {
        Files.delete(output);
      } catch (IOException exception) {
        System.err.println(exception);
      }
    }
    mStart = System.currentTimeMillis();
  }

  // Resolving type spends more time, so when execute time is too long,
  // we don't resolve type for fields. But declare will be resolved if possible.
  private boolean isLongTask() {
    return (System.currentTimeMillis() - mStart) > mTimeout;
  }

  /*
   * Set timeout value for visitor's parser.
   *
   * @Param timeout An timeout value (millisecond) for resolving type.
   *                Since it may spends more time to resolve object type, if
   *                elapsed time of visitor's parser is more than timeout
   *                value, we don't resolve type except to declare.
   *                Default value is 5 min.
   */
  public void setTimeout(long timeout) {
    mTimeout = timeout;
  }

  private static String getScope(final String fullName, final SimpleName name) {
    return fullName.substring(0, fullName.length() - name.toString().length());
  }

  private String getScopeOfParameterType(final Parameter parameter) {
    // Resolving type is expensive.
    if (isLongTask()) {
      return "";
    }

    try {
      return getScopeOfType(parameter.getType(), parameter.resolve().getType());
    } catch (Exception e) {
      // not resolved
    }
    return "";
  }

  private String getScopeOfType(final Type type, final ResolvedType resolvedType) {
    if (resolvedType == null) {
      return "";
    }

    if (type.isClassOrInterfaceType() && resolvedType.isReferenceType()) {
      return getScope(
          resolvedType.asReferenceType().getQualifiedName(),
          type.asClassOrInterfaceType().getName());
    }
    if (type.isArrayType() && resolvedType.isArray()) {
      return getScopeOfType(
          type.asArrayType().getComponentType(), resolvedType.asArrayType().getComponentType());
    }
    return "";
  }

  private Type getClassOrInterfaceType(final Type type) {
    Type realType = type;
    while (!realType.isClassOrInterfaceType()) {
      if (!realType.isArrayType()) {
        return null;
      }
      realType = realType.asArrayType().getComponentType();
    }
    return realType;
  }

  private Type getRealType(final Type type) {
    Type realType = getClassOrInterfaceType(type);
    if (realType != null) {
      return realType;
    }
    return type;
  }

  private static String getContext(final Node n) {
    try {
      Optional<Node> parent = n.getParentNode();
      while (parent.isPresent()) {
        if (parent.get() instanceof MethodDeclaration) {
          MethodDeclaration d = (MethodDeclaration) parent.get();
          final ResolvedMethodDeclaration decl = d.resolve();
          return decl.getQualifiedName();
        } else if (parent.get() instanceof ConstructorDeclaration) {
          final ConstructorDeclaration d = (ConstructorDeclaration) parent.get();
          final ResolvedReferenceTypeDeclaration decl = d.resolve().declaringType();
          return decl.getQualifiedName() + "." + d.getName();
        }
        parent = parent.get().getParentNode();
      }
    } catch (Exception e) {
      // not resolved
    }
    return "";
  }

  private void handleGenericsArguments(final Type type, final String context) {
    if (!type.isClassOrInterfaceType()) {
      return;
    }

    Optional<NodeList<Type>> args = type.asClassOrInterfaceType().getTypeArguments();
    if (!args.isPresent()) {
      return;
    }

    for (Type t : args.get()) {
      String typeScope = "";
      if (!isLongTask()) {
        try {
          final ResolvedType resolvedType = t.resolve();
          typeScope = getScopeOfType(t, resolvedType);
          if (typeScope.length() > 0) {
            t = getRealType(t);
          }
        } catch (Exception e) {
        }
      }
      outputSource(t, typeScope);
      outputTarget(t, typeScope, context);
    }
  }

  // Emit objects functions

  private void outputJSON(final JSONObject obj) {
    try {
      if (!Files.exists(mOutputPath.getParent())) {
        Files.createDirectories(mOutputPath.getParent());
      }
      PrintWriter printWriter =
          new PrintWriter(new BufferedWriter(new FileWriter(mOutputPath.toFile(), true)));
      printWriter.println(obj);
      printWriter.close();
    } catch (IOException exception) {
      System.err.println(exception);
    }
  }

  private void outputSource(final ClassOrInterfaceDeclaration n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final Type n, final String scope) {
    if (n.isClassOrInterfaceType()) {
      final ClassOrInterfaceType classType = n.asClassOrInterfaceType();
      outputSource(classType, scope);
    } else if (n.isArrayType()) {
      final Type typeInArray = n.asArrayType().getComponentType();
      outputSource(typeInArray, scope);
    }
  }

  private void outputSource(final ClassOrInterfaceType n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final ConstructorDeclaration n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final MethodDeclaration n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final VariableDeclarator n, final String scope, boolean isVariable) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    if (isVariable) {
      obj.put("no_crossref", 1);
    }
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final EnumDeclaration n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);
    outputJSON(obj);
  }

  private void outputSource(final EnumConstantDeclaration n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);
    outputJSON(obj);
  }

  private void outputSource(final ObjectCreationExpr n, final SimpleName name, final String scope) {
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final MethodCallExpr n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final FieldAccessExpr n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final Parameter n) {
    final Type type = n.getType();
    final String typeScope = getScopeOfParameterType(n);
    outputSource(type, typeScope);

    final SimpleName name = n.getName();
    final String fullName = name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, "");
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final ReferenceType n, final SimpleName name, final String scope) {
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputSource(final NameExpr n, final String scope) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addSourceLine(name).addSource(n, name, scope);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(
      final ClassOrInterfaceDeclaration n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final Type n, final String scope, final String context) {
    if (n.isClassOrInterfaceType()) {
      final ClassOrInterfaceType type = n.asClassOrInterfaceType();
      outputTarget(type, scope, context);
    } else if (n.isArrayType()) {
      final Type typeInArray = n.asArrayType().getComponentType();
      outputTarget(typeInArray, scope, context);
    }
  }

  private void outputTarget(
      final ClassOrInterfaceType n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(
      final ConstructorDeclaration n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final VariableDeclarator n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final EnumDeclaration n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(
      final EnumConstantDeclaration n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final MethodDeclaration n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(
      final ObjectCreationExpr n, final SimpleName name, final String scope, final String context) {
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final MethodCallExpr n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final FieldAccessExpr n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final NameExpr n, final String scope, final String context) {
    final SimpleName name = n.getName();
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(final Parameter n, final String context) {
    final Type type = n.getType();
    final String typeScope = getScopeOfParameterType(n);
    outputTarget(type, typeScope, context);

    final SimpleName name = n.getName();
    final String fullName = name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, "", context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  private void outputTarget(
      final ReferenceType n, final SimpleName name, final String scope, final String context) {
    final String fullName = scope + name.getIdentifier();

    MozSearchJSONObject obj = new MozSearchJSONObject();
    obj.addTargetLine(name).addTarget(n, name, scope, context);
    obj.addSymbol(fullName);

    outputJSON(obj);
  }

  // Declarations

  @Override
  public void visit(ClassOrInterfaceDeclaration n, String a) {
    String scope = "";
    String context = "";

    try {
      final ResolvedReferenceTypeDeclaration decl = n.resolve();
      scope = getScope(decl.getQualifiedName(), n.getName());
      if (scope.length() > 0) {
        context = scope.substring(0, scope.length() - 1);
      }
    } catch (Exception e) {
      // not resolved
    }

    outputSource(n, scope);
    outputTarget(n, scope, context);

    for (ClassOrInterfaceType classType : n.getExtendedTypes()) {
      String typeScope = "";
      try {
        typeScope = getScopeOfType(classType, classType.resolve());
      } catch (Exception e) {
      }
      outputSource(classType, typeScope);
      outputTarget(classType, typeScope, context);
    }
    for (ClassOrInterfaceType classType : n.getImplementedTypes()) {
      String typeScope = "";
      try {
        typeScope = getScopeOfType(classType, classType.resolve());
      } catch (Exception e) {
      }
      outputSource(classType, typeScope);
      outputTarget(classType, typeScope, context);
    }
    super.visit(n, a);
  }

  @Override
  public void visit(VariableDeclarator n, String a) {
    String scope = "";
    String context = "";
    boolean isVariable = false;
    ResolvedType resolvedType = null;

    if (!isLongTask()) {
      try {
        final ResolvedValueDeclaration decl = n.resolve();
        isVariable = decl.isVariable();
        if (decl.isField()) {
          final ResolvedTypeDeclaration typeDecl = decl.asField().declaringType();
          scope = typeDecl.getQualifiedName() + ".";
          context = typeDecl.getQualifiedName();
        } else {
          context = getContext(n);
        }
        resolvedType = decl.getType();
      } catch (Exception e) {
        // not resolved
      }
    }

    outputSource(n, scope, isVariable);
    outputTarget(n, scope, context);

    Type type = n.getType();
    String typeScope = getScopeOfType(type, resolvedType);
    if (typeScope.length() > 0) {
      type = getRealType(type);
    }
    outputSource(type, typeScope);
    outputTarget(type, typeScope, context);

    handleGenericsArguments(type, context);

    super.visit(n, a);
  }

  @Override
  public void visit(EnumDeclaration n, String a) {
    String scope = "";
    String context = "";

    try {
      final ResolvedEnumDeclaration decl = n.resolve();
      scope = getScope(decl.getQualifiedName(), n.getName());
      if (scope.length() > 0) {
        context = scope.substring(0, scope.length() - 1);
      }
    } catch (Exception e) {
      // not resolved
    }

    outputSource(n, scope);
    outputTarget(n, scope, context);

    if (scope.length() > 0) {
      context = scope + n.getName();
      scope = scope + n.getName() + ".";
    }
    for (EnumConstantDeclaration child : n.getEntries()) {
      outputSource(child, scope);
      outputTarget(child, scope, context);
    }

    super.visit(n, a);
  }

  @Override
  public void visit(ConstructorDeclaration n, String a) {
    String scope = "";
    String context = "";

    // Even if this analyze is too long, we resolve this.
    try {
      final ResolvedReferenceTypeDeclaration decl = n.resolve().declaringType();
      scope = decl.getQualifiedName() + ".";
      context = decl.getQualifiedName();
    } catch (Exception e) {
      // not resolved
    }

    outputSource(n, scope);
    outputTarget(n, scope, context);
    // output constructor name only too
    if (scope.length() > 0) {
      outputSource(n, "");
      outputTarget(n, "", context);
    }

    for (Parameter parameter : n.getParameters()) {
      outputSource(parameter);
      outputTarget(parameter, context);
    }

    super.visit(n, a);
  }

  @Override
  public void visit(MethodDeclaration n, String a) {
    String scope = "";
    String context = "";
    ResolvedType resolvedType = null;

    // Even if this analyze is too long, we resolve this.
    try {
      final ResolvedMethodDeclaration decl = n.resolve();
      scope = getScope(decl.getQualifiedName(), n.getName());
      if (scope.length() > 0) {
        context = scope.substring(0, scope.length() - 1);
      }
      resolvedType = decl.getReturnType();
    } catch (Exception e) {
      // not resolved
    }

    outputSource(n, scope);
    outputTarget(n, scope, context);
    // output method name only too
    if (scope.length() > 0) {
      outputSource(n, "");
      outputTarget(n, "", context);
    }

    // Output parameters
    for (Parameter parameter : n.getParameters()) {
      outputSource(parameter);
      outputTarget(parameter, context);
    }

    // exceptions
    for (ReferenceType exception : n.getThrownExceptions()) {
      String typeScope = "";
      try {
        typeScope = getScopeOfType(exception, exception.resolve());
      } catch (Exception e) {
        // not resolved
      }
      outputSource(exception, typeScope);
      outputTarget(exception, typeScope, context);
    }

    // return type
    Type type = n.getType();
    final String typeScope = getScopeOfType(type, resolvedType);
    if (typeScope.length() > 0) {
      type = getRealType(type);
    }
    outputSource(type, typeScope);
    outputTarget(type, typeScope, context);

    handleGenericsArguments(type, context);

    super.visit(n, a);
  }

  @Override
  public void visit(CatchClause n, String a) {
    final Parameter parameter = n.getParameter();
    final String context = getContext(n);

    outputSource(parameter);
    outputTarget(parameter, context);

    super.visit(n, a);
  }

  @Override
  public void visit(MethodCallExpr n, String a) {
    String scope = "";

    if (!isLongTask()) {
      try {
        final ResolvedMethodDeclaration decl = n.resolve();
        scope = getScope(decl.getQualifiedName(), n.getName());
      } catch (Exception e) {
        // not resolved.
      }
    }

    final String context = getContext(n);

    outputSource(n, scope);
    outputTarget(n, scope, context);

    super.visit(n, a);
  }

  @Override
  public void visit(NameExpr n, String a) {
    String scope = "";

    if (!isLongTask()) {
      try {
        final ResolvedValueDeclaration decl = n.resolve();
        if (decl.isField()) {
          final ResolvedTypeDeclaration typeDecl = decl.asField().declaringType();
          scope = typeDecl.getQualifiedName() + ".";
        }
      } catch (Exception e) {
        // not resolved
      }
    }

    final String context = getContext(n);

    outputSource(n, scope);
    outputTarget(n, scope, context);

    super.visit(n, a);
  }

  @Override
  public void visit(ObjectCreationExpr n, String a) {
    String scope = "";

    if (!isLongTask()) {
      try {
        final ResolvedConstructorDeclaration decl = n.resolve();
        scope = getScope(decl.getQualifiedName(), n.getType().getName());
      } catch (Exception e) {
        // not resolved
      }
    }

    final String context = getContext(n);

    outputSource(n, n.getType().getName(), scope);
    outputTarget(n, n.getType().getName(), scope, context);

    handleGenericsArguments(n.getType(), context);

    super.visit(n, a);
  }

  @Override
  public void visit(FieldAccessExpr n, String a) {
    String scope = "";

    if (!isLongTask()) {
      try {
        final ResolvedFieldDeclaration decl = n.resolve().asField();
        final ResolvedTypeDeclaration typeDecl = decl.declaringType();
        scope = typeDecl.getQualifiedName() + ".";
      } catch (Exception e) {
        // not resolved
      }
    }

    final String context = getContext(n);

    outputSource(n, scope);
    outputTarget(n, scope, context);

    super.visit(n, a);
  }

  @Override
  public void visit(CastExpr n, String a) {
    Type type = n.getType();
    String scope = "";

    // Resolving type is expensive
    if (!isLongTask()) {
      try {
        scope = getScopeOfType(type, type.resolve());
        if (scope.length() > 0) {
          type = getRealType(type);
        }
      } catch (Exception e) {
        // not resolved
      }
    }

    final String context = getContext(n);

    outputSource(type, scope);
    outputTarget(type, scope, context);

    super.visit(n, a);
  }
}
