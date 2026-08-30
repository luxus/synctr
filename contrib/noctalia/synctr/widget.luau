-- Bar widget. The only status contract is `synctr status --json`.
-- Fields read here must match StatusSnapshot in synctr-engine/src/status.rs:
--   rclone.found, rclone.path, rclone.source, rclone.detail
--   profiles[].name, profiles[].mode, profiles[].last_run.ok, last_run.exit_code

local function summarize(data)
	local rclone = data.rclone
	if type(rclone) ~= "table" or rclone.found ~= true then
		return "cloud-off", "no rclone", "synctr: rclone not found"
	end
	local profiles = data.profiles
	if type(profiles) ~= "table" or #profiles == 0 then
		return "cloud", "synctr", "synctr: no profiles"
	end
	local ok, fail, never = 0, 0, 0
	for i = 1, #profiles do
		local p = profiles[i]
		local last = p.last_run
		if last == nil then
			never = never + 1
		elseif last.ok == true then
			ok = ok + 1
		else
			fail = fail + 1
		end
	end
	if fail > 0 then
		return "cloud-exclamation", string.format("%d fail", fail),
			string.format("synctr: %d ok, %d fail, %d never", ok, fail, never)
	end
	return "cloud", string.format("%d ok", ok),
		string.format("synctr: %d ok, %d fail, %d never", ok, fail, never)
end

function update()
	noctalia.setUpdateInterval(10000)
	noctalia.runAsync("synctr status --json", function(result)
		if result.exitCode ~= 0 then
			barWidget.setGlyph("alert-circle")
			barWidget.setText("synctr")
			barWidget.setTooltip("synctr status --json failed")
			return
		end
		local data, err = noctalia.json.decode(result.stdout)
		if type(data) ~= "table" then
			barWidget.setGlyph("alert-circle")
			barWidget.setText("synctr")
			barWidget.setTooltip(err or "invalid json")
			return
		end
		local glyph, text, tip = summarize(data)
		barWidget.setGlyph(glyph)
		barWidget.setText(text)
		barWidget.setTooltip(tip)
	end)
end

function onClick()
	noctalia.runAsync("synctr status --json", function(result)
		if result.exitCode ~= 0 then
			noctalia.notify("synctr", "status --json failed")
			return
		end
		local data = noctalia.json.decode(result.stdout)
		if type(data) ~= "table" then
			noctalia.notify("synctr", "invalid json")
			return
		end
		local _, text, tip = summarize(data)
		noctalia.notify(text, tip)
	end)
end
