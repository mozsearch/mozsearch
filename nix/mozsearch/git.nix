# git, with a fix for git fast-import's scaling to repos with millions of
# distinct file names (see the patch), for the tools which write repos with
# git fast-import (ex: the history tools).  The tools find it via MOZSEARCH_GIT;
# see `fast_import_git` in tools/src/git_ops.rs.
{gitMinimal}:
gitMinimal.overrideAttrs (old: {
  pname = "mozsearch-git";
  patches = (old.patches or []) ++ [./git-fast-import-grow-atom-table.patch];
  # Only run git fast-import's tests, since that's all the patch changes.
  preInstallCheck =
    old.preInstallCheck
    + ''
      installCheckFlagsArray+=(T="$(cd t && echo t93??-fast-import*.sh)")
    '';
})
