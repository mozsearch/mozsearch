package org.mozilla.mozsearch;

import com.github.javaparser.ast.body.ClassOrInterfaceDeclaration;
import com.github.javaparser.ast.body.ConstructorDeclaration;
import com.github.javaparser.ast.body.EnumConstantDeclaration;
import com.github.javaparser.ast.body.EnumDeclaration;
import com.github.javaparser.ast.body.MethodDeclaration;
import com.github.javaparser.ast.body.Parameter;
import com.github.javaparser.ast.body.VariableDeclarator;
import com.github.javaparser.ast.expr.FieldAccessExpr;
import com.github.javaparser.ast.expr.MethodCallExpr;
import com.github.javaparser.ast.expr.NameExpr;
import com.github.javaparser.ast.expr.ObjectCreationExpr;
import com.github.javaparser.ast.expr.SimpleName;
import com.github.javaparser.ast.type.ClassOrInterfaceType;
import com.github.javaparser.ast.type.ReferenceType;
import org.json.JSONObject;

public class MozSearchJSONObject extends JSONObject {
  public MozSearchJSONObject() {
    super();
  }

  public MozSearchJSONObject addSourceLine(final SimpleName name) {
    put(
            "loc",
            name.getBegin().get().line
                + ":"
                + (name.getBegin().get().column - 1)
                + "-"
                + (name.getBegin().get().column - 1 + name.getIdentifier().length()))
        .put("source", 1);
    return this;
  }

  public MozSearchJSONObject addTargetLine(final SimpleName name) {
    put("loc", name.getBegin().get().line + ":" + (name.getBegin().get().column - 1))
        .put("target", 1);
    return this;
  }

  public MozSearchJSONObject addSymbol(final String symbolName) {
    put("sym", symbolName.replace('.', '#'));
    return this;
  }

  public JSONObject addSource(
      final ClassOrInterfaceDeclaration n, final SimpleName name, final String scope) {
    if (((ClassOrInterfaceDeclaration) n).isInterface()) {
      return put("syntax", "def,type").put("pretty", "interface " + scope + name.getIdentifier());
    }
    return put("syntax", "def,type").put("pretty", "class " + scope + name.getIdentifier());
  }

  public JSONObject addSource(
      final ClassOrInterfaceType n, final SimpleName name, final String scope) {
    return put("syntax", "type,use").put("pretty", "class " + scope + name.getIdentifier());
  }

  public JSONObject addSource(final ReferenceType n, final SimpleName name, final String scope) {
    return put("syntax", "type,use")
        .put("pretty", "class/interface/enum " + scope + name.getIdentifier());
  }

  public JSONObject addSource(
      final ConstructorDeclaration n, final SimpleName name, final String scope) {
    return put("syntax", "def,function")
        .put("pretty", "constructor " + scope + name.getIdentifier());
  }

  public JSONObject addSource(
      final MethodDeclaration n, final SimpleName name, final String scope) {
    return put("syntax", "def,function").put("pretty", "method " + scope + name.getIdentifier());
  }

  public JSONObject addSource(final Parameter n, final SimpleName name, final String scope) {
    return put("syntax", "use,variable")
        .put("pretty", "parameter " + scope + name.getIdentifier())
        .put("no_crossref", 1);
  }

  public JSONObject addSource(final EnumDeclaration n, final SimpleName name, final String scope) {
    return put("syntax", "def,variable").put("pretty", "enum " + scope + name.getIdentifier());
  }

  public JSONObject addSource(
      final EnumConstantDeclaration n, final SimpleName name, final String scope) {
    return put("syntax", "def,variable")
        .put("pretty", "enum constant " + scope + name.getIdentifier());
  }

  public JSONObject addSource(
      final VariableDeclarator n, final SimpleName name, final String scope) {
    if (scope.length() > 0) {
      return put("syntax", "def,variable").put("pretty", "member " + scope + name.getIdentifier());
    }
    return put("syntax", "use,variable").put("pretty", "variable " + scope + name.getIdentifier());
  }

  public JSONObject addSource(final MethodCallExpr n, final SimpleName name, final String scope) {
    return put("syntax", "use,function").put("pretty", "method " + scope + name.getIdentifier());
  }

  public JSONObject addSource(
      final ObjectCreationExpr n, final SimpleName name, final String scope) {
    return put("syntax", "use,function")
        .put("pretty", "constructor " + scope + name.getIdentifier());
  }

  public JSONObject addSource(final FieldAccessExpr n, final SimpleName name, final String scope) {
    return put("syntax", "use").put("pretty", "member " + scope + name.getIdentifier());
  }

  public JSONObject addSource(final NameExpr n, final SimpleName name, final String scope) {
    if (scope.length() > 0) {
      return put("syntax", "use,variable").put("pretty", "member " + scope + name.getIdentifier());
    }
    return put("syntax", "uselocal,variable")
        .put("pretty", "variable " + scope + name.getIdentifier())
        .put("no_crossref", 1);
  }

  public JSONObject addTarget(
      final ClassOrInterfaceDeclaration n,
      final SimpleName name,
      final String scope,
      final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final ClassOrInterfaceType n,
      final SimpleName name,
      final String scope,
      final String context) {
    return put("kind", "use").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final ReferenceType n, final SimpleName name, final String scope, final String context) {
    return put("kind", "use").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final ConstructorDeclaration n,
      final SimpleName name,
      final String scope,
      final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final MethodDeclaration n, final SimpleName name, final String scope, final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final EnumDeclaration n, final SimpleName name, final String scope, final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final EnumConstantDeclaration n,
      final SimpleName name,
      final String scope,
      final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final VariableDeclarator n, final SimpleName name, final String scope, final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final MethodCallExpr n, final SimpleName name, final String scope, final String context) {
    return put("kind", "use").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final ObjectCreationExpr n, final SimpleName name, final String scope, final String context) {
    return put("kind", "use").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final FieldAccessExpr n, final SimpleName name, final String scope, final String context) {
    return put("kind", "use").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final NameExpr n, final SimpleName name, final String scope, final String context) {
    return put("kind", "use").put("pretty", scope + name.getIdentifier()).put("context", context);
  }

  public JSONObject addTarget(
      final Parameter n, final SimpleName name, final String scope, final String context) {
    return put("kind", "def").put("pretty", scope + name.getIdentifier()).put("context", context);
  }
}
