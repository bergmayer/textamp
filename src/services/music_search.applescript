-- Music's search URLs can launch the app without submitting the query.
-- Use its catalog search UI instead. macOS controls Automation/Accessibility
-- permission; never change those settings or touch the user's clipboard.
on run argv
    set searchTerm to item 1 of argv
    tell application "Music" to activate
    tell application "System Events"
        tell process "Music"
            repeat 30 times
                if frontmost and (count of windows) > 0 then exit repeat
                delay 0.1
            end repeat
            if not frontmost then error "Music is not frontmost"
            if (count of windows) is 0 then error "Music window unavailable"
            keystroke "f" using command down
            set searchField to missing value
            set catalogScope to missing value
            repeat 30 times
                -- Cmd+F rebuilds the toolbar. Inspect this small subtree, not
                -- a transient focus reference or a large library/results tree.
                try
                    set uiItems to entire contents of toolbar 1 of front window
                    repeat with uiItem in uiItems
                        if role of uiItem is "AXTextField" then
                            if subrole of uiItem is "AXSearchField" then set searchField to contents of uiItem
                        else if role of uiItem is "AXRadioButton" then
                            if description of uiItem is "Apple Music" then set catalogScope to contents of uiItem
                        end if
                    end repeat
                    if searchField is not missing value and catalogScope is not missing value then exit repeat
                end try
                delay 0.1
            end repeat
            if searchField is missing value then error "Music search field unavailable"
            -- Never accidentally search only the local library. Unsupported
            -- layouts or unavailable catalog access use the web fallback.
            if catalogScope is missing value then error "Apple Music search scope unavailable"
            click catalogScope
            set value of searchField to searchTerm
            set focused of searchField to true
            if not frontmost then error "Music lost focus before search"
            if value of searchField is not searchTerm then error "Music did not accept the query"
            key code 36
        end tell
    end tell
end run
