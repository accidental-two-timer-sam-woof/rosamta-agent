
# >>> conda initialize >>>
# !! Contents within this block are managed by 'conda init' !!
if test -f @HOME@/miniforge3/bin/conda
    eval @HOME@/miniforge3/bin/conda "shell.fish" "hook" $argv | source
else
    if test -f "@HOME@/miniforge3/etc/fish/conf.d/conda.fish"
        . "@HOME@/miniforge3/etc/fish/conf.d/conda.fish"
    else
        set -x PATH "@HOME@/miniforge3/bin" $PATH
    end
end
# <<< conda initialize <<<

