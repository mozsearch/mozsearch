This file attempts to provide the basics of bash scripting as relevant to
searchfox's automation.  Note that _all_ scripts use bash, not sh, which means
we have additional tricks available.

Nothing here is authoritative or exhaustive.  If you want those, check out
these links:
- [Bash FAQ](https://mywiki.wooledge.org/BashFAQ) and specific pages:
  - [`Arguments`](https://mywiki.wooledge.org/Arguments)
  - [`Quotes`](https://mywiki.wooledge.org/Quotes)
  - [`What is the difference between test, [ and [[ ?`](https://mywiki.wooledge.org/BashFAQ/031)
  - [`How do I do string manipulations in bash?`](https://mywiki.wooledge.org/BashFAQ/100)
  - [`How can I use parameter expansion? (and more)!`](https://mywiki.wooledge.org/BashFAQ/073)

It's probably a good idea to read the
[Quotes](https://mywiki.wooledge.org/Quotes) and
[Arguments](https://mywiki.wooledge.org/Arguments) pages if you're touching
anything related to variables.

## Error Handling

We use the following initialization stanza in all bash scripts at the top:
```bash
set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline
```

## Argument Handling

Because we set that undefined variables are fatal, it's not okay to reference
a positional argument like `$4` unless it's a mandatory argument, and ideally
after checking the number of arguments.

### Checking the number of arguments

Exact match:
```bash
if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <ARG>"
    echo " e.g.: $0 example-arg"
    exit 1
fi
```

Minimum count:
```bash
if [[ $# -lt 2 ]]
then
    echo "Usage: $0 arg-1 arg-2 [optional-arg]"
    exit 1
fi
```

### Checking whether variables are empty

If `[[` is used instead of `[` there's no need to quote the variable.  Note that
because of `set -eu`, this will still error if the variable is not defined.  See
the next section for how to handle that.

```bash
if [[ $defined_var_that_may_be_empty ]]; then
    # logic to run if the variable was non-empty
fi
```

### Dealing with optional arguments

Default an argument to something if unset or empty (the `:` makes it handle
empty in addition to unset):
```bash
NAME=${4:-default_value}
```

This also works if you want to normalize an omitted argument to being empty:
```bash
NAME=${4:-}
```

## Checking the file-system

Check whether a file exists and is a regular file.

```bash
if [[ -f $PATH ]]; then
    # logic to run if the file existed
fi
```

Check if it doesn't exist or isn't a regular file.

```bash
if [[ ! -f $PATH ]]; then
    # logic to run if the file didn't exist / wasn't a file
fi
```

Other related tests:
* `-f` is a file (not a directory or something weird)
* `-x` is an executable file
* `-d` is a directory
* `-e` is any kind of file
* `-h` is a symlink

```bash
if [[ -d $PATH ]]; then
    # logic to run if the dir existed and was a dir
fi
```

Check if it doesn't exist or isn't a directory.

```bash
if [[ ! -d $PATH ]]; then
    # logic to run if the dir didn't exist or wasn't a file.
fi
```

If you don't care if something is a directory or weird thing, use `-e`.

## Commands and Escaping Arguments
It's still probably a good idea to read the
[Quotes](https://mywiki.wooledge.org/Quotes) and
[Arguments](https://mywiki.wooledge.org/Arguments) pages if you're touching
anything related to this.  But here are important highlights.

### Nesting quotes does not do what you think it does.

As documented at [Quoting Happens Before PE](https://mywiki.wooledge.org/Arguments#Quoting_Happens_Before_PE)
if you put single quotes inside a double quote to try and escape something that
you know will be passed to another shell invocation, the single quotes will be
escaped as content, which is probably not what you were trying to do.  Example:

```bash
testfoo='bar' # the use of single-quotes doesn't matter here
set -x # Show commands
$ echo "'$testfoo'"
+ echo ''\''bar'\''' #
'bar'
```

If you're thinking about doing this because you're using `parallel`, see the
section on `parallel`.

### Double-quotes stops globbing but not variable expansion.
Using a wildcard that you don't want globbed because you're passing it to
`find`?  Wrap it in double-quotes, and you can still use variables!
```bash
"*.json foo-${VAR}-*.json"
```

### For loops are bad - while loops are good

See https://mywiki.wooledge.org/BashFAQ/001 but the basic idea is that instead
of doing:

```bash
# THIS IS THE BAD EXAMPLE DON'T DO THIS BECAUSE IF THERE ARE SPACES IN THE FILE
# NAME IT WILL BE PARSED AS TWO SEPARATE TOKENS, NOT ONE, AND THEN YOU WILL HAVE
# A BAD TIME.
for file in $(find . -type f | sort -r); do
  gzip -f "$file"
  touch -r "$file".gz "$file"
done
```

you want to do:

```bash
find . -type f | sort -r | while read -r file; do
  gzip -f "$file"
  touch -r "$file".gz "$file"
done
```

because the for loop will tokenize things incorrectly.

### GNU parallel and its processing

GNU parallel does use a shell in each of its invocations.  So shell parsing
will happen both in the invocation of parallel and each of its sub-invocations.

Passing `-q` to parallel will cause it to escape everything it passes to the
shell.  This is necessary in cases where arguments contain characters like `;`
which the shell will interpret and aren't automatically escaped by parallel.

The `-q` option should be used instead of attempting to embed quotes within
quotes, which https://mywiki.wooledge.org/Arguments#Quoting_Happens_Before_PE
tells us will end badly.

Parallel has a `--shellquote` argument that can be used to generate a quoted
version of a parallel command so that `-q` doesn't need to be used (which could
preclude some shell magic).

See https://www.gnu.org/software/parallel/parallel_tutorial.html#Quoting for
more info.