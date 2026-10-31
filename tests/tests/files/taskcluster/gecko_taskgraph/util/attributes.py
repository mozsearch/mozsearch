# The projects tasks can run on: `run-on-projects` names projects, or aliases
# for some of them.

TRUNK_PROJECTS = {
    "autoland",
    "mozilla-central",
}

RELEASE_PROJECTS = {
    "mozilla-central",
    "mozilla-beta",
    "mozilla-release",
}

RUN_ON_PROJECT_ALIASES = {
    "all": lambda project: True,
    "trunk": lambda project: project in TRUNK_PROJECTS,
    "release": lambda project: project in RELEASE_PROJECTS,
}


def match_run_on_projects(project, run_on_projects):
    for alias in run_on_projects:
        if alias in RUN_ON_PROJECT_ALIASES:
            if RUN_ON_PROJECT_ALIASES[alias](project):
                return True
        elif alias == project:
            return True
    return False
