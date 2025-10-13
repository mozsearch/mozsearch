This directory contains one-off manually generated versions of metadata
that we download from taskcluster for firefox-main and other gecko
repositories.

Right now it's somewhat of a feature that the files only cover a small
subset of the files in the tests repo, each with a distinct permutation
(ex: different test information configurations).  If we attempt to make
these files more comprehensive or dynamically generated, it would probably
be appropriate to create specific test files that somehow self-describe to
the auto-generation machinery what sort of data they want in their source.
(Embedding the description in the source makes it easier to sanity check
when viewing the searchfox page for the file.)
