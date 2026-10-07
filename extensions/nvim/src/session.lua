local filename, recovery, channel = ...
vim.o.modeline = false
vim.o.exrc = false
vim.o.directory = recovery .. '//'
vim.o.updatecount = 1
vim.o.updatetime = 200
vim.o.mouse = 'a'
vim.o.mousemodel = 'popup_setpos'
vim.o.termguicolors = true
-- nvim_cmd takes a literal filename argument: no Ex interpolation.
vim.api.nvim_cmd({cmd='edit', args={filename}}, {})
vim.bo.readonly = false
vim.bo.modifiable = true
local function modified()
  for _, b in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_valid(b) and vim.bo[b].modified then return true end
  end
  return false
end
local function state()
  vim.rpcnotify(channel, 'starfold_editor', modified())
  if vim.bo.modified then pcall(vim.cmd, 'preserve') end
end
local function attach()
  if vim.b.starfold_attached then return end
  vim.b.starfold_attached = true
  vim.api.nvim_buf_attach(0, false, {on_lines=function()
    vim.rpcnotify(channel, 'starfold_editor', true)
  end})
end
local group = vim.api.nvim_create_augroup('StarfoldEditor', {clear=true})
vim.api.nvim_create_autocmd('BufEnter', {group=group, callback=function() attach(); state() end})
vim.api.nvim_create_autocmd({'TextChanged','TextChangedI','BufModifiedSet','BufWritePost'}, {group=group, callback=state})
attach()
state()
vim.cmd('redraw!')
