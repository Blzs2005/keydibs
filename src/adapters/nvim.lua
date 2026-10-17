-- Run by keydibs inside `nvim --headless`, after your config has loaded.
-- Prints the keymap of every mode, plus `:help index` for the built-in
-- commands, as JSON on a line starting with "KEYDIBS ".

local function dump()
  -- lazy.nvim and other plugin managers set up some keys only once a UI is
  -- attached (lazy.nvim's VeryLazy event), so pretend one is, and give them a
  -- moment to finish.
  vim.api.nvim_exec_autocmds("UIEnter", {})
  vim.wait(100)

  local scripts = {}
  for _, script in ipairs(vim.fn.getscriptinfo()) do
    scripts[script.sid] = script.name
  end

  local maps = {}
  for _, mode in ipairs({ "n", "i", "x", "s", "o", "c", "t" }) do
    for _, map in ipairs(vim.api.nvim_get_keymap(mode)) do
      local script = scripts[map.sid]
      table.insert(maps, {
        mode = mode,
        lhs = vim.fn.keytrans(map.lhsraw or map.lhs),
        desc = map.desc,
        rhs = map.rhs,
        script = script,
        -- Nvim's own Lua code is compiled in, so it has no file.
        exists = script ~= nil and (vim.uv or vim.loop).fs_stat(script) ~= nil,
        line = map.lnum,
      })
    end
  end

  local runtime = vim.env.VIMRUNTIME
  local ok, index = pcall(vim.fn.readfile, runtime .. "/doc/index.txt")
  io.stdout:write("\nKEYDIBS " .. vim.json.encode({
    runtime = runtime,
    maps = maps,
    index = ok and table.concat(index, "\n") or nil,
  }) .. "\n")
end

-- Scheduled, so it runs once startup is over (after VimEnter).
vim.schedule(function()
  local ok, err = pcall(dump)
  if not ok then
    io.stderr:write(tostring(err) .. "\n")
  end
  vim.cmd("qa!")
end)
