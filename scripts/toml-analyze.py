#!/usr/bin/env python3

import sys
import os
import re
import json


def at_escape(text):
  return re.sub("[^A-Za-z0-9_/]", lambda m: "@" + "{:02X}".format(ord(m.group(0))), text)


def to_file_sym(filename):
    return "FILE_" + at_escape(filename)


def to_loc(line, c1, c2):
    return f"{line:05d}:{c1}-{c2}"


class ParseError(Exception):
    def __init__(self, parser, message):
        start = parser.i
        while start > 0:
            if parser.text[start - 1] in ["\r", "\n"]:
                break
            start -= 1

        tlen = len(parser.text)
        end = parser.i
        while end < tlen:
            if parser.text[end] in ["\r", "\n"]:
                break
            end += 1

        column = parser.i - start

        super().__init__(f"{message} at {parser.path}:{parser.lineno}:{parser.column}\n\n" +
                         parser.text[start:end] + "\n" +
                         (" " * column) + "^")


# https://github.com/toml-lang/toml/blob/8eae5e1c005bc5836098505f85a7aa06568999dd/toml.abnf
class Parser:
    def __init__(self, path, text, callback):
        self.path = path
        self.text = text
        self.i = 0
        self.lineno = 1
        self.column = 0
        self.callback = callback

    def match(self, prefix):
        if self.text.startswith(prefix, self.i):
            # print("match", prefix)
            consumed = len(prefix)
            self.i += consumed
            self.column += consumed
            return True
        return False

    def peek(self, prefix):
        if self.text.startswith(prefix, self.i):
            return True
        return False

    def rmatch(self, pattern):
        m = re.match(pattern, self.text[self.i:])
        if m:
            # print("rmatch", pattern, m.group(0))
            consumed = len(m.group(0))
            self.i += consumed
            self.column += consumed
            return m
        return None

    def on_newline(self):
        self.lineno += 1
        self.column = 0

    def eof(self):
        return self.i == len(self.text)

    def pos(self):
        return [self.lineno, self.column]

    # toml = expression *( newline expression )
    def toml(self):
        self.expression()

        while self.maybe_newline():
            self.expression()

        if not self.eof():
            raise ParseError(self, "Unexpected garbage after body")

    # expression =  ws [ comment ]
    # expression =/ ws keyval ws [ comment ]
    # expression =/ ws table ws [ comment ]
    #
    # table = std-table / array-table
    def expression(self):
        self.ws()
        if self.match("[["):
            self.array_table()
            self.ws()
            self.maybe_comment()
        elif self.match("["):
            self.std_table()
            self.ws()
            self.maybe_comment()
        elif self.match("#"):
            self.comment_cont()
        elif self.eof():
            return
        elif self.peek("\r\n") or self.peek("\n"):
            return
        else:
            [ks, v] = self.keyval()
            self.callback.on_keyval(ks, v)
            self.ws()
            self.maybe_comment()

    # ws = *wschar
    # wschar =  %x20  ; Space
    # wschar =/ %x09  ; Horizontal tab
    def ws(self):
        self.rmatch(r"^[ \t]*")

    # newline =  %x0A     ; LF
    # newline =/ %x0D.0A  ; CRLF
    def maybe_newline(self):
        m = self.rmatch(r"^\r?\n")
        if m:
            self.on_newline()
        return m

    # comment-start-symbol = %x23 ; #
    # non-ascii = %x80-D7FF / %xE000-10FFFF
    # non-eol = %x09 / %x20-7F / non-ascii
    #
    # comment = comment-start-symbol *non-eol
    def maybe_comment(self):
        if self.match("#"):
            self.comment_cont()

    def comment_cont(self):
        self.rmatch(r"^[^\x00-\x08\x0A-\x1F\x7F]*")

    # keyval = key keyval-sep val
    # keyval-sep = ws %x3D ws ; =
    def keyval(self):
        ks = self.key()
        self.ws()
        if not self.match("="):
            raise ParseError(self, "Expected =")
        self.ws()
        v = self.val()
        return [ks, v]

    # key = simple-key / dotted-key
    # simple-key = quoted-key / unquoted-key
    #
    # unquoted-key = 1*( ALPHA / DIGIT / %x2D / %x5F ) ; A-Z / a-z / 0-9 / - / _
    # quoted-key = basic-string / literal-string
    # dotted-key = simple-key 1*( dot-sep simple-key )
    #
    # dot-sep   = ws %x2E ws  ; . Period
    #
    # ALPHA = %x41-5A / %x61-7A ; A-Z / a-z
    # DIGIT = %x30-39 ; 0-9
    def key(self):
        ks = []
        while True:
            start = self.pos()
            if self.match('"'):
                s = self.basic_string()
                end = self.pos()
                ks.append(["string", s, start, end])
            elif self.match("'"):
                s = self.literal_string()
                end = self.pos()
                ks.append(["string", s, start, end])
            else:
                m = self.rmatch(r"^[A-Za-z0-9-_]+")
                if not m:
                    raise ParseError(self, "Expected key")
                end = self.pos()
                ks.append(["unquoted", m.group(0), start, end])
            self.ws()
            if self.match("."):
                self.ws()
                continue
            break
        return ks

    # val = string / boolean / array / inline-table / date-time / float / integer
    #
    # string = ml-basic-string / basic-string / ml-literal-string / literal-string
    #
    # boolean = true / false
    #
    # true    = %x74.72.75.65     ; true
    # false   = %x66.61.6C.73.65  ; false
    def val(self):
        start = self.pos()
        if self.match('"""'):
            s = self.ml_basic_string()
            end = self.pos()
            return ["string", s, start, end]
        if self.match("'''"):
            s = self.ml_literal_string()
            end = self.pos()
            return ["string", s, start, end]
        if self.match('"'):
            s = self.basic_string()
            end = self.pos()
            return ["string", s, start, end]
        if self.match("'"):
            s = self.literal_string()
            end = self.pos()
            return ["string", s, start, end]
        if self.match("["):
            a = self.array()
            return ["array", a]
        if self.match("{"):
            self.inline_table()
            return None
        if self.match("true"):
            return None
        if self.match("false"):
            return None
        if self.maybe_date_time():
            return None
        if self.maybe_float_or_integer():
            return None
        raise ParseError(self, "Expected val")

    # basic-string = quotation-mark *basic-char quotation-mark
    #
    # quotation-mark = %x22            ; "
    #
    # basic-char = basic-unescaped / escaped
    # basic-unescaped = wschar / %x21 / %x23-5B / %x5D-7E / non-ascii
    # escaped = escape escape-seq-char
    #
    def basic_string(self):
        s = ""
        while True:
            if self.match("\\"):
                 s = self.escaped()
            elif self.match('"'):
                break
            else:
                m = self.rmatch(r"^[^\x00-\x08\x0A-\x1F\x22\x5C\x7F]")
                if not m:
                    raise ParseError(self, "Expected basic-unescaped")
                s += m.group(0)
        return s


    # escape = %x5C                   ; \
    # escape-seq-char =  %x22         ; "    quotation mark  U+0022
    # escape-seq-char =/ %x5C         ; \    reverse solidus U+005C
    # escape-seq-char =/ %x62         ; b    backspace       U+0008
    # escape-seq-char =/ %x66         ; f    form feed       U+000C
    # escape-seq-char =/ %x6E         ; n    line feed       U+000A
    # escape-seq-char =/ %x72         ; r    carriage return U+000D
    # escape-seq-char =/ %x74         ; t    tab             U+0009
    # escape-seq-char =/ %x75 4HEXDIG ; uXXXX                U+XXXX
    # escape-seq-char =/ %x55 8HEXDIG ; UXXXXXXXX            U+XXXXXXXX
    #
    # HEXDIG = DIGIT / "A" / "B" / "C" / "D" / "E" / "F"
    def escaped(self):
        if self.match("u"):
            m = self.rmatch(r"^[0-9A-Fa-f]{4}")
            if not m:
                raise ParseError(self, "Expected uXXXX")
            return chr(int(m.group(0), 16))
        if self.match("U"):
            m = self.rmatch(r"^[0-9A-Fa-f]{8}")
            if not m:
                raise ParseError(self, "Expected UXXXXXXXX")
            return chr(int(m.group(0), 16))
        if self.match('"'):
            return '"'
        if self.match("\\"):
            return "\\"
        if self.match("b"):
            return "\b"
        if self.match("f"):
            return "\f"
        if self.match("n"):
            return "\n"
        if self.match("r"):
            return "\r"
        if self.match("t"):
            return "\t"
        raise ParseError(self, "Unknown escape")

    # ml-basic-string = ml-basic-string-delim [ newline ] ml-basic-body
    #                   ml-basic-string-delim
    # ml-basic-string-delim = 3quotation-mark
    # ml-basic-body = *mlb-content *( mlb-quotes 1*mlb-content ) [ mlb-quotes ]
    #
    # mlb-content = mlb-char / newline / mlb-escaped-nl
    # mlb-char = mlb-unescaped / escaped
    # mlb-quotes = 1*2quotation-mark
    # mlb-unescaped = wschar / %x21 / %x23-5B / %x5D-7E / non-ascii
    # mlb-escaped-nl = escape ws newline *( wschar / newline )
    def ml_basic_string(self):
        self.maybe_newline()

        quote_count = 0
        while True:
            if self.match('"'):
                quote_count += 1
                if quote_count == 3:
                    # There can be at most 2 more quotes
                    self.match('"')
                    self.match('"')
                    break
                continue
            quote_count = 0

            if self.match("\\"):
                # escaped / mlb-escaped-nl
                m = self.rmatch(r"^[ \t]*\r?\n")
                if m:
                    self.on_newline()

                    # mlb-escaped-nl
                    while True:
                        if self.rmatch(r"^[ \t]"):
                            pass
                        elif self.maybe_newline():
                            pass
                        break
                else:
                    # escaped
                    self.escaped()
            elif self.maybe_newline():
                pass
            else:
                m = self.rmatch(r"^[^\x00-\x08\x0A-\x1F\x22\x5C\x7F]")
                if not m:
                    raise ParseError(self, "Expected mlb-unescaped")

    # literal-string = apostrophe *literal-char apostrophe
    #
    # apostrophe = %x27 ; ' apostrophe
    #
    # literal-char = %x09 / %x20-26 / %x28-7E / non-ascii
    def literal_string(self):
        s = ""
        while True:
            if self.match("'"):
                break
            else:
                m = self.rmatch(r"^[^\x00-\x08\x0A-\x1F\x27\x7F]")
                if not m:
                    raise ParseError(self, "Expected literal-char")
                s += m.group(0)
        return s

    # ml-literal-string = ml-literal-string-delim [ newline ] ml-literal-body
    #                     ml-literal-string-delim
    # ml-literal-string-delim = 3apostrophe
    # ml-literal-body = *mll-content *( mll-quotes 1*mll-content ) [ mll-quotes ]
    #
    # mll-content = mll-char / newline
    # mll-char = %x09 / %x20-26 / %x28-7E / non-ascii
    # mll-quotes = 1*2apostrophe
    def ml_literal_string(self):
        self.maybe_newline()

        quote_count = 0
        while True:
            if self.match("'"):
                quote_count += 1
                if quote_count == 3:
                    # There can be at most 2 more quotes
                    self.match("'")
                    self.match("'")
                    break
                continue
            quote_count = 0

            if self.maybe_newline():
                pass
            else:
                m = self.rmatch(r"^[^\x00-\x08\x0A-\x1F\x27\x7F]")
                if not m:
                    raise ParseError(self, "Expected mlb-unescaped")

    # integer = dec-int / hex-int / oct-int / bin-int
    #
    # minus = %x2D                       ; -
    # plus = %x2B                        ; +
    # underscore = %x5F                  ; _
    # digit1-9 = %x31-39                 ; 1-9
    # digit0-7 = %x30-37                 ; 0-7
    # digit0-1 = %x30-31                 ; 0-1
    #
    # hex-prefix = %x30.78               ; 0x
    # oct-prefix = %x30.6F               ; 0o
    # bin-prefix = %x30.62               ; 0b
    #
    # dec-int = [ minus / plus ] unsigned-dec-int
    # unsigned-dec-int = DIGIT / digit1-9 1*( DIGIT / underscore DIGIT )
    #
    # hex-int = hex-prefix HEXDIG *( HEXDIG / underscore HEXDIG )
    # oct-int = oct-prefix digit0-7 *( digit0-7 / underscore digit0-7 )
    # bin-int = bin-prefix digit0-1 *( digit0-1 / underscore digit0-1 )
    #
    # float = float-int-part ( exp / frac [ exp ] )
    # float =/ special-float
    #
    # float-int-part = dec-int
    # frac = decimal-point zero-prefixable-int
    # decimal-point = %x2E               ; .
    # zero-prefixable-int = DIGIT *( DIGIT / underscore DIGIT )
    #
    # exp = "e" float-exp-part
    # float-exp-part = [ minus / plus ] zero-prefixable-int
    #
    # special-float = [ minus / plus ] ( inf / nan )
    # inf = %x69.6e.66  ; inf
    # nan = %x6e.61.6e  ; nan
    def maybe_float_or_integer(self):
        if self.rmatch(r"^0x[0-9A-Fa-f](_?[0-9A-Fa-f])*"):
            # hex-int
            return True
        if self.rmatch(r"^0o[0-7](_?[0-7])*"):
            # oct-int
            return True
        if self.rmatch(r"^0b[01](_?[01])*"):
            # bin-int
            return True
        if self.rmatch(r"^[+-]?(0|[1-9](_?[0-9])*)"):
            # dec-int / float
            if self.rmatch(r"^[eE][+-]?[0-9](_?[0-9])*"):
                pass
            elif self.rmatch(r"^\.[0-9](_?[0-9])*([eE][+-]?[0-9](_?[0-9])*)?"):
                pass
            return True
        if self.rmatch(r"^[+-](inf|nan)"):
            # special-float
            return True
        return False

    # date-time      = offset-date-time / local-date-time / local-date / local-time
    #
    # time-delim     = "T" / %x20 ; T, t, or space
    #
    # full-time      = partial-time time-offset
    #
    # offset-date-time = full-date time-delim full-time
    # local-date-time = full-date time-delim partial-time
    # local-date = full-date
    # local-time = partial-time
    def maybe_date_time(self):
        if self.maybe_full_date():
            # offset-date-time / local-date-time / local-date
            if self.rmatch("^[Tt ]"):
                if self.maybe_partial_time():
                    self.maybe_time_offset()
                else:
                    raise ParseError(self, "Expected partial-time")
            return True
        if self.maybe_partial_time():
            # local-time
            return True
        return False

    # date-fullyear  = 4DIGIT
    # date-month     = 2DIGIT  ; 01-12
    # date-mday      = 2DIGIT  ; 01-28, 01-29, 01-30, 01-31 based on month/year
    #
    # full-date      = date-fullyear "-" date-month "-" date-mday
    def maybe_full_date(self):
        return self.rmatch(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}")

    # time-hour      = 2DIGIT  ; 00-23
    # time-minute    = 2DIGIT  ; 00-59
    # time-second    = 2DIGIT  ; 00-58, 00-59, 00-60 based on leap second rules
    # time-secfrac   = "." 1*DIGIT
    #
    # partial-time   = time-hour ":" time-minute ":" time-second [ time-secfrac ]
    def maybe_partial_time(self):
        return self.rmatch(r"^[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?")

    # time-numoffset = ( "+" / "-" ) time-hour ":" time-minute
    # time-offset    = "Z" / time-numoffset
    def maybe_time_offset(self):
        if self.rmatch(r"^[Zz]"):
            return True
        if self.rmatch(r"^[+-][0-9]{2}:[0-9]{2}"):
            return True
        return False

    # array = array-open [ array-values ] ws-comment-newline array-close
    #
    # array-open =  %x5B ; [
    # array-close = %x5D ; ]
    #
    # array-values =  ws-comment-newline val ws-comment-newline array-sep array-values
    # array-values =/ ws-comment-newline val ws-comment-newline [ array-sep ]
    #
    # array-sep = %x2C  ; , Comma
    def array(self):
        a = []
        while True:
            self.ws_comment_newline()
            if self.match("]"):
                break
            v = self.val()
            a.append(v)
            self.ws_comment_newline()
            if self.match(","):
                self.ws_comment_newline()
        return a

    # ws-comment-newline = *( wschar / [ comment ] newline )
    def ws_comment_newline(self):
        while True:
            if self.rmatch(r"^(#[^\x00-\x08\x0A-\x1F\x7F]*)?\r?\n"):
                self.on_newline()
                continue
            if self.rmatch(r"^[ \t]+"):
                continue
            break


    # std-table = std-table-open key std-table-close
    #
    # std-table-open  = %x5B ws     ; [ Left square bracket
    # std-table-close = ws %x5D     ; ] Right square bracket
    def std_table(self):
        self.ws()
        ks = self.key()
        if len(ks) > 0:
            self.callback.on_std_table(ks)
        self.ws()
        self.match("]")

    # inline-table = inline-table-open [ inline-table-keyvals ] inline-table-close
    #
    # inline-table-open  = %x7B ws     ; {
    # inline-table-close = ws %x7D     ; }
    # inline-table-sep   = ws %x2C ws  ; , Comma
    #
    # inline-table-keyvals = keyval [ inline-table-sep inline-table-keyvals ]
    def inline_table(self):
        self.ws()
        if self.match("}"):
            return

        while True:
            self.keyval()
            self.ws()
            if self.match(","):
                self.ws()
                continue
            if self.match("}"):
                break
            else:
                raise ParseError(self, "Expected inline-table-close")

    # array-table = array-table-open key array-table-close
    #
    # array-table-open  = %x5B.5B ws  ; [[ Double left square bracket
    # array-table-close = ws %x5D.5D  ; ]] Double right square bracket
    def array_table(self):
        self.ws()
        self.key()
        self.ws()
        self.match("]]")


class AnalysisWriter:
    def __init__(self, local_path, analysis_path):
        self.test_dir = os.path.dirname(local_path)
        self.analysis_path = analysis_path
        self.items = []

        self.items.append({
            "loc": "00001:0",
            "target": 1,
            "kind": "def",
            "pretty": local_path,
            "sym": to_file_sym(local_path),
        })

    def add_use(self, filename, line, c1, c2):
        path = os.path.normpath(os.path.join(self.test_dir, filename))
        self.items.append({
            "loc": to_loc(line, c1, c2),
            "target": 1,
            "kind": "use",
            "pretty": path,
            "sym": to_file_sym(path),
        })
        self.items.append({
            "loc": to_loc(line, c1, c2),
            "source": 1,
            "syntax": "use,file",
            "pretty": path,
            "sym": to_file_sym(path),
        })

    def write(self):
        with open(self.analysis_path, "w") as f:
            for item in self.items:
                print(json.dumps(item), file=f)

    def on_keyval(self, ks, v):
        if len(ks) == 1 and ks[0][1] == "support-files":
            if v[0] == "array":
                for k in v[1]:
                    self.add_use(k[1], k[2][0], k[2][1], k[3][1])
            elif v[0] == "string":
                self.add_use(v[1], v[2][0], v[2][1], v[3][1])

    def on_std_table(self, ks):
        if len(ks) != 1:
            return
        k = ks[0]
        if k[0] != "string":
            return

        s = k[1]
        if s.startswith("include:"):
            self.add_use(s[8:], k[2][0], k[2][1], k[3][1])
        else:
            self.add_use(s, k[2][0], k[2][1], k[3][1])


def analyze(local_path, files_root, analysis_root):
    toml_path = os.path.join(files_root, local_path)
    analysis_path = os.path.join(analysis_root, local_path)

    with open(toml_path, "r") as f:
        text = f.read()
    w = AnalysisWriter(local_path, analysis_path)
    p = Parser(local_path, text, w)
    try:
        p.toml()
    except ParseError as e:
        # print("WARNING: " + str(e))
        return
    w.write()


index_root = sys.argv[1]
files_root = sys.argv[2]
analysis_root = sys.argv[3]

for local_path in sys.stdin:
    local_path = local_path.strip()
    analyze(local_path, files_root, analysis_root)
