// Interactive visual study. All files and operations are illustrative values.
const $ = selector => document.querySelector(selector);
const terminal = $('.terminal');
const sourceFiles = [
  ['..', '—', '—', 'Parent', '↰'],
  ['Exports', '—', '03 Oct', 'Folder', '▰'],
  ['RAW', '—', '04 Oct', 'Folder', '▰'],
  ['alpine-lake.jpg', '24.8 MB', '04 Oct', 'JPEG', '◇'],
  ['blue-hour.jpg', '23.8 MB', '04 Oct', 'JPEG', '◇'],
  ['contact-sheet.pdf', '2.1 MB', '05 Oct', 'PDF', '▤'],
  ['field-notes.md', '4.2 KB', '05 Oct', 'Text', '≡'],
  ['ridge-panorama.tiff', '128 MB', '04 Oct', 'TIFF', '◇'],
  ['trail-recording.flac', '7.7 MB', '04 Oct', 'FLAC', '♪'],
  ['waypoints.gpx', '12.4 KB', '04 Oct', 'GPX', '≡'],
];
const destinationFiles = [
  ['..', '—', '—', 'Parent', '↰'],
  ['Summer', '—', '18 Sep', 'Folder', '▰'],
  ['Winter', '—', '21 Feb', 'Folder', '▰'],
  ['readme.md', '1.2 KB', '01 Oct', 'Text', '≡'],
];
const projectFiles = [
  ['..', '—', '—', 'Parent', '↰'],
  ['staramp', '—', '09 Oct', 'Folder', '▰'],
  ['starcord', '—', '08 Oct', 'Folder', '▰'],
  ['starfold', '—', '09 Oct', 'Folder', '▰'],
  ['starkit', '—', '09 Oct', 'Folder', '▰'],
  ['README.md', '8.2 KB', '08 Oct', 'Text', '≡'],
  ['flake.nix', '3.4 KB', '09 Oct', 'Text', '≡'],
];
const makePane = (path, files, cursor, marks = []) => ({path, files, cursor, marks: new Set(marks), filter: '', history: []});
const workspaces = [
  {name: 'FIELD STUDIES', active: 0, panes: [makePane('~/Pictures/Field studies', sourceFiles, 3, [3, 4]), makePane('~/Archive/2026', destinationFiles, 1)]},
  {name: 'PROJECTS', active: 0, panes: [makePane('~/projects/star', projectFiles, 3), makePane('~/Archive/Projects', destinationFiles, 1)]},
];
let workspace = 0;
let yanked = [];
let expanded = false;
let paused = false;
let overlay = null;
let priorFocus = null;
let placeCursor = 0;
let waiting = [
  {name: 'blue-hour.jpg', detail: 'Copy · preserve file times'},
  {name: 'field-notes.md', detail: 'Copy · preserve file times'},
];
const currentWorkspace = () => workspaces[workspace];
const activePane = () => currentWorkspace().panes[currentWorkspace().active];
const say = message => { $('#command-message').textContent = message; };
const visibleIndices = pane => pane.files.flatMap((file, index) => file[0].toLowerCase().includes(pane.filter.toLowerCase()) ? [index] : []);
const selectedIndices = pane => [...(pane.marks.size ? pane.marks : new Set([pane.cursor]))].filter(index => index !== 0);
const sizes = {KB: 1000, MB: 1000000};
function markedSize(pane) {
  let total = 0;
  for (const index of pane.marks) {
    const [value, unit] = pane.files[index][1].split(' ');
    total += (Number(value) || 0) * (sizes[unit] || 0);
  }
  return total >= 1000000 ? (total / 1000000).toFixed(1) + ' MB' : total >= 1000 ? (total / 1000).toFixed(1) + ' KB' : '0 B';
}
function drawDigits(count) {
  const patterns = ['abcdef', 'bc', 'abdeg', 'abcdg', 'bcfg', 'acdfg', 'acdefg', 'abc', 'abcdefg', 'abcdfg'];
  const display = $('#marked-digits');
  display.replaceChildren();
  display.setAttribute('aria-label', count + ' marked files');
  for (const number of String(count).padStart(2, '0')) {
    const digit = document.createElement('span');
    digit.className = 'digit';
    digit.setAttribute('aria-hidden', 'true');
    for (const segment of 'abcdefg') {
      const part = document.createElement('i');
      part.className = 'segment ' + segment + (patterns[Number(number)].includes(segment) ? ' on' : '');
      digit.append(part);
    }
    display.append(digit);
  }
}
function draw() {
  const state = currentWorkspace();
  state.panes.forEach((pane, side) => {
    const region = $(side ? '#right-pane' : '#left-pane');
    const container = $(side ? '#destination-files' : '#files');
    const oldScroll = container.scrollTop;
    region.classList.toggle('active-pane', side === state.active);
    region.querySelector('.pane-path > span').textContent = pane.path;
    region.querySelector('.pane-path > b').textContent = side === state.active ? 'ACTIVE' : 'DESTINATION';
    const indices = visibleIndices(pane);
    for (const row of [...container.children]) {
      if (!indices.includes(Number(row.dataset.index))) row.remove();
    }
    for (const [position, index] of indices.entries()) {
      const file = pane.files[index];
      const row = container.querySelector(`[data-index="${index}"]`) || document.createElement('button');
      row.dataset.index = index;
      row.className = 'file-row' + (pane.cursor === index ? ' selected' : '');
      row.tabIndex = -1;
      row.title = file[0];
      row.setAttribute('aria-label', file[0] + (pane.marks.has(index) ? ', marked' : ''));
      row.setAttribute('aria-pressed', String(pane.cursor === index));
      [pane.marks.has(index) ? '✓' : '', file[0], file[1], file[2], file[3]].forEach((value, column) => {
        const span = row.children[column] || document.createElement('span');
        if (column === 1) {
          span.className = 'file-name';
          const icon = document.createElement('i');
          icon.textContent = file[4];
          span.replaceChildren(icon, document.createTextNode(value));
        } else span.textContent = value;
        if (!span.parentElement) row.append(span);
      });
      row.onclick = () => { state.active = side; pane.cursor = index; draw(); terminal.focus({preventScroll: true}); };
      row.ondblclick = () => { state.active = side; pane.cursor = index; openEntry(); };
      if (container.children[position] !== row) container.insertBefore(row, container.children[position] || null);
    }
    container.scrollTop = oldScroll;
    const summary = region.querySelector('.list-summary');
    summary.children[0].textContent = pane.marks.size + ' marked' + (pane.filter ? ' · /' + pane.filter : '');
    summary.children[0].style.color = 'var(--green)';
    summary.children[1].textContent = (pane.files.length - 1) + ' entries';
  });
  const pane = activePane();
  const side = state.active ? 'RIGHT' : 'LEFT';
  const commander = terminal.classList.contains('commander');
  const mode = commander ? 'COMMANDER' : 'FOLD';
  $('#active-label').textContent = side + ' PANE · ' + mode;
  $('#active-path').textContent = pane.path;
  $('#browser-title').textContent = mode + ' · ' + state.name;
  $('#selection-readout').textContent = pane.marks.size + ' marked · ' + markedSize(pane) + ' · ' + (pane.files.length - 1) + ' entries';
  $('#marked-size').textContent = markedSize(pane);
  $('#route-readout').textContent = side + ' → ' + (state.active ? 'LEFT' : 'RIGHT') + ' · ' + state.panes[1 - state.active].path;
  $('.command-location').textContent = (state.active ? 'R' : 'L') + ':' + String(pane.cursor + 1).padStart(2, '0') + ' / ' + pane.files.length;
  $('.windowbar > span').lastChild.textContent = ' local · workspace ' + String(workspace + 1).padStart(2, '0');
  drawDigits(pane.marks.size);
  updatePreview();
}
function revealCursor() {
  const list = $(currentWorkspace().active ? '#destination-files' : '#files');
  const selected = list.querySelector('.selected');
  if (!selected) return;
  const row = selected.getBoundingClientRect();
  const viewport = list.getBoundingClientRect();
  if (row.bottom > viewport.bottom) list.scrollTop += row.bottom - viewport.bottom;
  if (row.top < viewport.top) list.scrollTop -= viewport.top - row.top;
}
function updatePreview() {
  const pane = activePane();
  const file = pane.files[pane.cursor];
  $('#preview-title').textContent = file[0].toUpperCase();
  $('#detail-name').textContent = file[0];
  $('#detail-size').textContent = file[1];
  $('#detail-date').textContent = file[2] === '—' ? '—' : file[2] + ' 2026';
  $('.marked-indicator').textContent = pane.marks.has(pane.cursor) ? '● Marked for transfer' : '○ Unmarked';
  const image = ['JPEG', 'TIFF'].includes(file[3]);
  $('#image-stage').hidden = !image;
  $('#document-stage').hidden = image;
  $('#detail-type').textContent = image ? file[3] + ' · 6000 × 4000 · sRGB' : file[3] === 'Folder' ? 'Directory · enter to browse' : file[3] === 'Parent' ? 'Parent directory' : file[3] + ' · sample metadata';
  $('#document-stage pre').textContent = file[3] === 'Text' ? '# ' + file[0] + '\n\nBlue hour at the lake.\nTake less. Look longer.' : file[3] === 'Folder' ? '/ ' + file[0] + '\n\nDirectory\nenter to open' : file[3] === 'Parent' ? '../\n\nh to go up' : file[0] + '\n\n' + file[3] + ' · ' + file[1];
}
function switchPane() {
  if (!terminal.classList.contains('commander')) return;
  currentWorkspace().active = 1 - currentWorkspace().active;
  draw();
  say((currentWorkspace().active ? 'Right' : 'Left') + ' pane · ' + activePane().marks.size + ' marked · ' + (yanked.length ? yanked.length + ' yanked' : 'ready'));
}
function navigate(delta) {
  const pane = activePane();
  const indices = visibleIndices(pane);
  if (!indices.length) return;
  pane.cursor = indices[Math.max(0, Math.min(indices.length - 1, indices.indexOf(pane.cursor) + delta))];
  draw();
  revealCursor();
}
function openEntry() {
  const pane = activePane();
  const file = pane.files[pane.cursor];
  if (file[3] === 'Parent') { parent(); return; }
  if (file[3] !== 'Folder') { expanded = true; setExpanded(); say('Inspecting ' + file[0] + ' · i to fold'); return; }
  pane.history.push({path: pane.path, files: pane.files, cursor: pane.cursor});
  pane.path += '/' + file[0];
  pane.files = [['..', '—', '—', 'Parent', '↰'], ['notes.md', '1.2 KB', '05 Oct', 'Text', '≡'], ['study-01.jpg', '12 MB', '05 Oct', 'JPEG', '◇']];
  pane.cursor = 1;
  pane.marks.clear();
  pane.filter = '';
  draw();
  say('Opened ' + file[0] + ' · h to return');
}
function parent() {
  const pane = activePane();
  const previous = pane.history.pop();
  if (!previous) { say('Sample root · choose a folder to explore'); return; }
  Object.assign(pane, previous);
  pane.marks.clear();
  pane.filter = '';
  draw();
  say('Returned to parent directory');
}
function setExpanded() {
  $('#preview-panel').classList.toggle('expanded', expanded);
  $('#preview-toggle').replaceChildren(document.createTextNode(expanded ? 'fold ' : 'expand '));
  const key = document.createElement('kbd'); key.textContent = 'i'; $('#preview-toggle').append(key);
  $('#preview-toggle').setAttribute('aria-expanded', String(expanded));
}
function drawQueue() {
  $('.operations').classList.toggle('paused', paused);
  $('#queue-state').textContent = '1 ' + (paused ? 'paused' : 'active') + ' · ' + waiting.length + ' waiting';
  $('#queue-count').replaceChildren(document.createTextNode(String(waiting.length + 1).padStart(2, '0') + ' '));
  const items = document.createElement('small'); items.textContent = 'items'; $('#queue-count').append(items);
  $('#operation-status').textContent = paused ? 'PAUSED' : 'COPYING';
  $('#transfer-speed').replaceChildren(document.createTextNode((paused ? '0' : '42') + ' '));
  const unit = document.createElement('small'); unit.textContent = 'MB/s'; $('#transfer-speed').append(unit);
  $('#remaining-time').textContent = paused ? '--:--' : '00:02';
  document.querySelectorAll('.queue-row:not(.current)').forEach(row => row.remove());
  for (const entry of waiting) {
    const row = document.createElement('div'); row.className = 'queue-row';
    const info = document.createElement('div');
    const name = document.createElement('b'); name.textContent = entry.name;
    const detail = document.createElement('small'); detail.textContent = entry.detail;
    const state = document.createElement('span'); state.className = 'waiting'; state.textContent = 'WAITING';
    info.append(name, detail); row.append(info, state); $('.queue-list').append(row);
  }
}
function clearWaiting() { waiting = []; drawQueue(); say('Waiting requests cleared · active transfer retained'); }
function toggleView() {
  const commander = terminal.classList.toggle('commander');
  currentWorkspace().active = 0;
  $('#view-button').textContent = commander ? 'COMMANDER' : 'FOLD';
  $('#view-button').classList.toggle('pressed', commander);
  $('#view-button').setAttribute('aria-pressed', String(commander));
  draw();
  say(commander ? 'Commander · two independent panes' : 'Fold · single directory view');
}
const places = [
  {path: '/home/bstar', files: projectFiles},
  {path: '~/Pictures/Field studies', files: sourceFiles},
  {path: '~/projects/star', files: projectFiles},
  {path: '/home/bstar', files: projectFiles},
  {path: '/run/media/bstar/Field Archive', files: destinationFiles},
];
function openOverlay(name) {
  priorFocus = document.activeElement;
  overlay = name;
  $('#' + name + '-modal').hidden = false;
  if (name === 'places') { $('#places-input').value = ''; filterPlaces(); $('#places-input').focus(); }
  else if (name === 'filter') { $('#filter-input').value = activePane().filter; $('#filter-input').focus(); }
  else $('#close-help').focus();
}
function closeOverlay() {
  if (!overlay) return;
  $('#' + overlay + '-modal').hidden = true;
  overlay = null;
  (priorFocus || terminal).focus({preventScroll: true});
}
function visiblePlaces() { return [...document.querySelectorAll('.place')].filter(button => !button.hidden); }
function highlightPlace() {
  document.querySelectorAll('.place').forEach(button => button.classList.remove('selected'));
  visiblePlaces()[placeCursor]?.classList.add('selected');
}
function filterPlaces() {
  const query = $('#places-input').value.toLowerCase();
  document.querySelectorAll('.place').forEach(button => { button.hidden = !button.textContent.toLowerCase().includes(query); });
  placeCursor = 0;
  highlightPlace();
}
function choosePlace(button) {
  const index = [...document.querySelectorAll('.place')].indexOf(button);
  if (index < 0) return;
  const pane = activePane();
  Object.assign(pane, {path: places[index].path, files: places[index].files, cursor: 1, filter: '', history: []});
  pane.marks.clear();
  draw(); closeOverlay(); say('Opened ' + pane.path);
}
function handleKey(key, ctrl = false) {
  const pane = activePane();
  if (key === 'Tab') { switchPane(); return true; }
  if (key === 'j' || key === 'ArrowDown') { navigate(1); return true; }
  if (key === 'k' || key === 'ArrowUp') { navigate(-1); return true; }
  if (key === 'Home' || key === 'End') {
    const indices = visibleIndices(pane);
    if (indices.length) { pane.cursor = key === 'Home' ? indices[0] : indices.at(-1); draw(); revealCursor(); }
    return true;
  }
  if (key === ' ') {
    if (pane.cursor !== 0) { pane.marks.has(pane.cursor) ? pane.marks.delete(pane.cursor) : pane.marks.add(pane.cursor); draw(); say(pane.marks.size + ' marked in ' + (currentWorkspace().active ? 'right' : 'left') + ' pane'); }
    return true;
  }
  if (['Enter', 'l', 'ArrowRight'].includes(key)) { openEntry(); return true; }
  if (['h', 'ArrowLeft', 'Backspace'].includes(key)) { parent(); return true; }
  if (key === 'y') {
    yanked = selectedIndices(pane).map(index => ({name: pane.files[index][0], source: pane.path}));
    say('Yanked ' + yanked.length + ' entries · tab to destination, then p');
    return true;
  }
  if (key === 'p' || key === 'm') {
    const entries = key === 'p' ? yanked : selectedIndices(pane).map(index => ({name: pane.files[index][0], source: pane.path}));
    if (!entries.length) { say('Nothing yanked · use y first'); return true; }
    const destination = key === 'p' ? pane.path : currentWorkspace().panes[1 - currentWorkspace().active].path;
    waiting.push(...entries.map(entry => ({name: entry.name, detail: (key === 'p' ? 'Copy' : 'Move') + ' → ' + destination})));
    drawQueue(); say((key === 'p' ? 'Copy' : 'Move') + ' queued · ' + entries.length + ' entries → ' + destination);
    return true;
  }
  if (key === 'i') { expanded = !expanded; setExpanded(); return true; }
  if (key === 'v') { toggleView(); return true; }
  if (key === 'b' || key === 'e') { openOverlay('places'); return true; }
  if (key === '/') { openOverlay('filter'); return true; }
  if (key === '?') { openOverlay('help'); return true; }
  if ((key === 'x' && ctrl) || key === 'X') { paused = key === 'x'; drawQueue(); say(paused ? 'Queue paused · X resumes' : 'Queue resumed'); return true; }
  if (key === 'Escape') {
    if (pane.filter) { pane.filter = ''; draw(); say('Filter cleared'); } else clearWaiting();
    return true;
  }
  return false;
}
terminal.addEventListener('keydown', event => {
  if (overlay) {
    if (event.key === 'Escape') {
      event.preventDefault();
      if (overlay === 'filter') { activePane().filter = ''; draw(); }
      closeOverlay(); return;
    }
    if (overlay === 'places' && ['ArrowDown', 'ArrowUp', 'Enter'].includes(event.key)) {
      event.preventDefault(); const visible = visiblePlaces();
      if (event.key === 'Enter') { if (visible[placeCursor]) choosePlace(visible[placeCursor]); }
      else { placeCursor = Math.max(0, Math.min(visible.length - 1, placeCursor + (event.key === 'ArrowDown' ? 1 : -1))); highlightPlace(); }
      return;
    }
    if (overlay === 'filter' && event.key === 'Enter') { event.preventDefault(); closeOverlay(); say('Filter active · esc to clear'); return; }
    if (event.key === 'Tab') {
      const controls = [...$('#' + overlay + '-modal').querySelectorAll('button,input')].filter(control => !control.hidden);
      const index = controls.indexOf(document.activeElement);
      event.preventDefault(); controls[(index + (event.shiftKey ? -1 : 1) + controls.length) % controls.length]?.focus();
    }
    return;
  }
  if (event.target.closest('button') && ['Enter', ' '].includes(event.key)) return;
  if ((event.ctrlKey && event.key !== 'x') || event.metaKey || event.altKey) return;
  if (handleKey(event.key, event.ctrlKey)) event.preventDefault();
});
for (const name of ['places', 'help', 'filter']) {
  $('#' + name + '-button').onclick = () => openOverlay(name);
  $('#close-' + name).onclick = closeOverlay;
  $('#' + name + '-modal').onclick = event => { if (event.target.id === name + '-modal') closeOverlay(); };
}
$('#places-input').oninput = filterPlaces;
document.querySelectorAll('.place').forEach(button => { button.onclick = () => choosePlace(button); });
$('#filter-input').oninput = event => {
  const pane = activePane(); pane.filter = event.target.value;
  const visible = visibleIndices(pane);
  if (!visible.includes(pane.cursor) && visible.length) pane.cursor = visible[0];
  draw();
};
$('#preview-toggle').onclick = () => { expanded = !expanded; setExpanded(); terminal.focus({preventScroll: true}); };
document.querySelectorAll('[data-key]').forEach(button => {
  button.onclick = () => {
    if (button.dataset.pane !== undefined) currentWorkspace().active = Number(button.dataset.pane);
    terminal.focus({preventScroll: true}); handleKey(button.dataset.key); draw();
  };
});
document.querySelectorAll('[data-workspace]').forEach(button => {
  button.onclick = () => {
    workspace = Number(button.dataset.workspace);
    if (!terminal.classList.contains('commander')) currentWorkspace().active = 0;
    document.querySelectorAll('[data-workspace]').forEach(tab => { tab.classList.toggle('pressed', tab === button); tab.setAttribute('aria-pressed', String(tab === button)); });
    draw(); say('Workspace ' + (workspace + 1) + ' · ' + currentWorkspace().name.toLowerCase()); terminal.focus({preventScroll: true});
  };
});
document.querySelectorAll('.themes button').forEach(button => {
  button.onclick = () => {
    terminal.dataset.theme = button.dataset.theme;
    document.querySelectorAll('.themes button').forEach(option => { option.classList.toggle('active', option === button); option.setAttribute('aria-pressed', String(option === button)); });
  };
});
$('#stop-button').onclick = () => { handleKey('x', true); terminal.focus({preventScroll: true}); };
$('#clear-button').onclick = () => { clearWaiting(); terminal.focus({preventScroll: true}); };
$('#view-button').onclick = () => { toggleView(); terminal.focus({preventScroll: true}); };
$('#home-button').onclick = () => {
  const pane = activePane(); Object.assign(pane, {path: '/home/bstar', files: projectFiles, cursor: 1, filter: '', history: []}); pane.marks.clear();
  draw(); say('Opened sample home'); terminal.focus({preventScroll: true});
};
$('#reload-button').onclick = () => { draw(); say('Sample listing refreshed'); terminal.focus({preventScroll: true}); };
$('#focus').onclick = () => {
  const focused = document.body.classList.toggle('focused');
  $('#focus').textContent = focused ? 'EXIT FOCUS' : 'FOCUS';
  $('#focus').setAttribute('aria-pressed', String(focused));
  terminal.focus({preventScroll: true});
};
draw();
drawQueue();
