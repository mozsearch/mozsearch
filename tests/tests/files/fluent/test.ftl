# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

-brand-short-name = Searchfox
    .gender = feminine

## Tabs

# Variables:
#   $count (Number) - The number of tabs.
tabs-close-warning =
    { $count ->
        [one] Close { $count } tab of { -brand-short-name }?
       *[other] Close { $count } tabs of { -brand-short-name }?
    }
    .title = { tabs-close-button.tooltip }

tabs-close-button = Close
    .tooltip = Close the tab
    .accesskey = C

-https = https://{ $host }
about-link = See { -https(host: "searchfox.org") } for more.
