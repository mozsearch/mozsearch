$(function() {
    /**
     * Toggles the ARIA expanded and hidden attributes' state.
     *
     * @param Object elem The element on which to toggle the attribute.
     */
    function toggleAria(elem) {
        var expandedState = elem.attr('aria-expanded') === 'true' ? 'false' : 'true';
        var hiddenState = elem.attr('aria-hidden') === 'true' ? 'false' : 'true';
        elem.attr('aria-hidden', hiddenState);
        elem.attr('aria-expanded', expandedState);
    }

    $('#panel-toggle').click(function(event) {
        var panelContent = $('#panel-content');
        var icon = $('.navpanel-icon', this);

        icon.toggleClass('expanded');
        panelContent.toggle();
        toggleAria(panelContent);
    });

    var permalinkNode = $('.panel a[title="Permalink"]');
    permalinkNode = permalinkNode.length == 0 ? null : permalinkNode[0];

    function pushPermalink() {
        if (permalinkNode) {
            if (window.location.href != permalinkNode.href) {
                history.pushState(
                    { permalink: permalinkNode.href },
                    window.title,
                    permalinkNode.href
                );
            }
            return true;
        }
        return false;
    }

    if (permalinkNode) {
        permalinkNode.addEventListener('click', (event) => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
                return;
            }
            if (pushPermalink()) {
                event.preventDefault();
            }
        });
    }

    function handleAccelerator(event) {
        if (event.altKey || event.ctrlKey || event.metaKey) {
            return;
        }
        var inputs = /input|select|textarea/i;
        if (inputs.test(event.target.nodeName)) {
            return;
        }
        switch (event.key) {
            case 'y':
            case 'Y':
                if (pushPermalink()) {
                    event.preventDefault();
                }
                break;
            case 'l':
            case 'L':
                var linkNode = $('.panel a[title="Log"]');
                if (linkNode.length) {
                    linkNode[0].click();
                    event.preventDefault();
                }
                break;
            case 'r':
            case 'R':
                var linkNode = $('.panel a[title="Raw"]');
                if (linkNode.length) {
                    linkNode[0].click();
                    event.preventDefault();
                }
                break;
        }
    }

    function acceleratorsEnabledInLocalStorage() {
        return !('accel-enable' in localStorage) || localStorage.getItem('accel-enable') == '1';
    }

    var acceleratorsEnabled = acceleratorsEnabledInLocalStorage();

    if (acceleratorsEnabled) {
        // Keyboard accelerators are enabled, so register them.
        document.documentElement.addEventListener('keypress', handleAccelerator);
    } else {
        // Keyboard accelerators disabled, so reflect that state in the checkbox and
        // hide the accelerators. Also don't register the keyboard listeners.
        let panelAccelEnable = $('#panel-accel-enable')[0];
        if (panelAccelEnable) {
            panelAccelEnable.checked = false;
        }
        $('.panel span.accel').hide();
    }

    function updateAccelerators(newState) {
        if (acceleratorsEnabled == newState) {
            return;
        }
        acceleratorsEnabled = newState;
        let panelAccelEnable = $('#panel-accel-enable')[0];
        if (newState) {
            document.documentElement.addEventListener('keypress', handleAccelerator);
            $('.panel span.accel').show();
            localStorage.setItem('accel-enable', '1');
            if (panelAccelEnable) {
                panelAccelEnable.checked = true;
            }
        } else {
            document.documentElement.removeEventListener('keypress', handleAccelerator);
            $('.panel span.accel').hide();
            localStorage.setItem('accel-enable', '0');
            if (panelAccelEnable) {
                panelAccelEnable.checked = false;
            }
        }
    }

    // If the user toggles the checkbox let's update the state accordingly.
    let panelAccelEnable = document.getElementById('panel-accel-enable');
    if (panelAccelEnable) {
        panelAccelEnable.addEventListener('change', () => {
            var newState = (panelAccelEnable.checked);
            updateAccelerators(newState);
        });
    }
    // If the user toggles it in a different tab, update the checkbox/state here
    window.addEventListener("storage", function() {
        var newState = acceleratorsEnabledInLocalStorage();
        updateAccelerators(newState);
    });
});
