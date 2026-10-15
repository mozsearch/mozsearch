/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this file,
 * You can obtain one at http://mozilla.org/MPL/2.0/. */

import React from "react";
import { useSelector } from "react-redux";
import {
  WIDGET_REGISTRY,
  isWidgetEnabled,
  resolveWidgetSize,
  resolveWidgetHasSidebar,
} from "common/WidgetsRegistry.mjs";
import { Weather as WeatherWidget } from "./Weather/Weather";
import { ErrorBoundary } from "content-src/components/ErrorBoundary/ErrorBoundary";
import { WIDGET_SIDEBAR_COMPONENTS } from "./WidgetsComponentRegistry.jsx";

const PREF_WIDGETS_ENABLED = "widgets.enabled";
const PREF_NOVA_ENABLED = "nova.enabled";

function WidgetsSidebar({ dispatch }) {
  const prefs = useSelector(state => state.Prefs.values);
  const widgetsEnabled = prefs[PREF_WIDGETS_ENABLED];
  const novaEnabled = prefs[PREF_NOVA_ENABLED];

  // Only one widget is supported here going forward: .sidebar-inline-end
  // subgrids two rows of the outer grid, and anything past that overflows into
  // the .content-full-width band below it.
  const sidebarWidgets = WIDGET_REGISTRY.filter(
    w =>
      resolveWidgetHasSidebar(w, prefs) &&
      isWidgetEnabled(w, prefs, widgetsEnabled) &&
      resolveWidgetSize(w, prefs) === "small"
  );

  if (!sidebarWidgets.length) {
    return null;
  }

  return (
    <>
      {sidebarWidgets.map(w => {
        if (novaEnabled) {
          const Component = WIDGET_SIDEBAR_COMPONENTS[w.id];
          return Component ? (
            <ErrorBoundary key={w.id}>
              <Component dispatch={dispatch} />
            </ErrorBoundary>
          ) : null;
        }
        // @nova-cleanup: remove below
        if (w.id === "weather") {
          if (!prefs.showWeather) {
            return null;
          }
          return (
            <ErrorBoundary key="weather">
              <WeatherWidget dispatch={dispatch} size="small" />
            </ErrorBoundary>
          );
        }
        return null;
      })}
    </>
  );
}

export { WidgetsSidebar };
