# Target tasks methods: the functions a decision task selects its tasks with,
# by the name .cron.yml's jobs give as their `target-tasks-method`.

_target_task_methods = {}


def register_target_task(name):
    def wrap(func):
        _target_task_methods[name] = func
        return func

    return wrap


@register_target_task("package_nightly")
def target_tasks_package_nightly(full_task_graph, parameters, graph_config):
    """Select the nightly package and the toolchain it's built with."""
    return ["package-linux64/opt", "toolchain-linux64-clang-19"]


@register_target_task("nothing")
def target_tasks_nothing(full_task_graph, parameters, graph_config):
    """Select nothing."""
    return []
