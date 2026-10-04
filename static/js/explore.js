/**
 * The /explore/ pages' sparklines (see explore.liquid): a cursor follows the
 * mouse across all of the sparklines at the commit under it, and the status
 * line and the commit list say which commit it is.
 */
(function () {
  const root = document.querySelector(".explore-page");
  if (!root) {
    return;
  }
  const status = root.querySelector(".explore-status");
  const defaultStatus = status?.innerHTML;
  const commits = [...root.querySelectorAll(".explore-commit")];
  const slot = parseInt(root.dataset.slot, 10);
  let current = -1;

  function commitAt(event) {
    const svg = event.target.closest?.("svg.explore-sparkline");
    if (!svg) {
      return -1;
    }
    const index = Math.floor((event.clientX - svg.getBoundingClientRect().left) / slot);
    return index < commits.length ? index : -1;
  }

  root.addEventListener("mousemove", event => {
    const index = commitAt(event);
    if (index == current) {
      return;
    }
    commits[current]?.classList.remove("explore-current");
    current = index;
    if (index < 0) {
      root.style.removeProperty("--explore-cursor");
      if (status) {
        status.innerHTML = defaultStatus;
      }
      return;
    }
    root.style.setProperty("--explore-cursor", `${index * slot}px`);
    const commit = commits[index];
    commit.classList.add("explore-current");
    if (status) {
      status.innerHTML = `Commit ${index + 1}: ` +
        commit.querySelector(".explore-commit-summary").innerHTML +
        ` <a href="#${commit.id}">(in the list)</a>`;
    }
  });

  // The commit list may be collapsed.
  root.addEventListener("click", event => {
    const link = event.target.closest?.('a[href^="#commit-"]');
    if (link) {
      root.querySelector(".explore-commit-list").open = true;
    }
  });

  // The commits checked as side A and side B of an interdiff (see
  // `format_interdiff` in format.rs) make the picker's link.
  const interdiffLink = root.querySelector(".explore-interdiff-link");
  function updateInterdiffLink() {
    const picked = side =>
      [...root.querySelectorAll(`.explore-pick-${side}:checked`)].map(box => box.value);
    const [a, b] = [picked("a"), picked("b")];
    if (a.length && b.length) {
      const tree = document.getElementById("data").getAttribute("data-tree");
      interdiffLink.href = `/${tree}/interdiff/${a.join(",")}/${b.join(",")}`;
    } else {
      interdiffLink.removeAttribute("href");
    }
  }
  root.addEventListener("change", event => {
    if (event.target.matches?.(".explore-pick-a, .explore-pick-b")) {
      updateInterdiffLink();
    }
  });
  if (interdiffLink) {
    updateInterdiffLink();
  }
})();
