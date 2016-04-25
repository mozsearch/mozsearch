git filter-branch -f --commit-filter '
  TREE=$(shift)

  FILES=$(git show $GIT_COMMIT | grep ^\+\+\+ | cut -c 7-)

  for FILE in $FILES
  do
    git blame -l -s  $GIT_COMMIT -- $FILE | cut -c 1-40 > $FILE
  done
' HEAD
