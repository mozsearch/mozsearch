# Contributing to Searchfox

## Filing issues/bugs

Please file bugs in
[Bugzilla](https://bugzilla.mozilla.org/enter_bug.cgi?product=Webtools&component=Searchfox).
We do not accept bug reports on GitHub (and the issues tab is disabled
for this reason).

## Submitting patches

Before writing a patch, please
[file a bug](https://bugzilla.mozilla.org/enter_bug.cgi?product=Webtools&component=Searchfox)
describing the problem you wish to solve. See the documentation in the
[README.md](README.md) file on how to set up a local development environment
with which you can test your changes. We realize the process of setting up a
local dev environment is quite cumbersome and can be a deterrent; so for
small patches that you're fairly confident in, you may just skip to
submitting a PR and include a note that you haven't tested it. In this
scenario one of the project maintainers can test it for you before merging,
or ping you to address any issues discovered.

When creating a PR, please ensure your changes are grouped into separate
commits logically, so that they are easy to review. The commits that are
functionally involved in fixing the bug should be tagged with the bug number
from Bugzilla (i.e. `Bug 12345 - Summary of commit` should be the first line
of the commit message). Commits that are doing cleanup/refactoring that are
not strictly related to the fix may omit the bug number.

After creating the PR, please comment on the bug with a link to the PR.
Project maintainers get notifications of all activity and should review
the patch soon. If you don't get a response within a few days, feel free
to ping @staktrace on the PR or needinfo :kats on the bug.
