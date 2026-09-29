# git, with a fix for git fast-import's scaling to repos with millions of
# distinct file names (see the patch), for the tools which write repos with
# git fast-import (ex: the history tools).  The tools find it via MOZSEARCH_GIT;
# see `fast_import_git` in tools/src/git_ops.rs.
{
  gitMinimal,
  openssl,
}:
gitMinimal.overrideAttrs (old: {
  pname = "mozsearch-git";
  patches = (old.patches or []) ++ [./git-fast-import-grow-atom-table.patch];
  # OpenSSL's SHA-1 (which uses the CPU's SHA instructions) rather than git's
  # collision-detecting one, which was ~15% of git fast-import's time for
  # build-timeline-tree on firefox.  We only hash what the tools derive from
  # the source repos, whose own objects are checked by the git that fetches
  # them.
  makeFlags = (old.makeFlags or []) ++ ["OPENSSL_SHA1=YesPlease"];
  buildInputs = (old.buildInputs or []) ++ [openssl];
  # Only run git fast-import's tests, since that's all the patch changes.
  preInstallCheck =
    old.preInstallCheck
    + ''
      installCheckFlagsArray+=(T="$(cd t && echo t93??-fast-import*.sh)")
    '';
})
