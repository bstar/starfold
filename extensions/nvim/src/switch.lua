local filename = ...
for _, b in ipairs(vim.api.nvim_list_bufs()) do
  if vim.api.nvim_buf_is_valid(b) and vim.bo[b].modified then
    error('Unsaved editor changes')
  end
end
vim.api.nvim_cmd({cmd='edit', args={filename}}, {})
-- Preview navigation replaces clean file buffers. Modified buffers and files
-- displayed in user-created splits keep their normal Neovim ownership.
local current = vim.api.nvim_get_current_buf()
for _, b in ipairs(vim.api.nvim_list_bufs()) do
  if b ~= current and vim.api.nvim_buf_is_valid(b) and vim.bo[b].buftype == ''
      and not vim.bo[b].modified and #vim.fn.win_findbuf(b) == 0 then
    vim.api.nvim_buf_delete(b, {})
  end
end
vim.cmd('redraw!')
