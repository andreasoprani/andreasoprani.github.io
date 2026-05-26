+++
title = "Milky Way Pool Club"
description = "A tiny space billiards game with gravity."
date = 2026-05-26
+++

Recently, I made a small pool-like game with gravity as a twist: [Milky Way Pool Club](/mwpc/).

The game started as a follow-up to [Pikuma's 2D Game Physics Programming course](https://pikuma.com/courses/game-physics-engine-programming) (which I recommend).
I took the main physics system from the course which was implemented in C++ with SDL2, ported it to C with raylib and in the process I simplified it a little bit since my logic didn't need to handle polygon-polygon contacts (which were the most complex ones).

The rest of the logic is pretty straight-forward, but it's the first game I fully implement myself (and without a game engine) so it was a fun exercise.

The planet sprites are cut out from the Balatro planet cards by [LocalThunk](https://localthunk.com/).
I like the card sprites (and the game) very much, but I can replace them with other sprites if it's an issue.

I published this so that I can consider this project done and move on to a new one, even though there are still TODOs in the repo that I may address in the future.

Anyway, you can [play it here](/mwpc/) and you can [view the source code here](https://github.com/andreasoprani/mwpc).
