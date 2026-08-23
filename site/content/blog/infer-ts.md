+++
title = "infer-ts"
description = ""
date = 2026-08-14
draft = true
+++

# infer-ts: you can just do stuff

## A disclaimer

This post doesn't have a single clear purpose, it is the textual version of a talk (TODO) that discussed multiple thoughts that came to me during and after the development of a small library that I needed for work.

It has, I would say, 4 main motives:

1. Show & Tell: I will elaborate on what problem the library solves, and how it does so.
2. Technical: while we talk about the project, we will touch on the various tools it relies on (Polars, PyO3, Rust & Python).
3. Exploratory: the project is (almost) entirely "vibe-coded", in my personal version of the term. We will explore a bit what that means, especially in these kinds of greenfield projects.
4. Motivational: agentic coding has lowered the friction of developing these kinds of projects, you can just do stuff!

So, I apologize if what follows is a little bit all over the place, but bear with me if any of the above points interest you.

Btw, you can find the code [here](https://github.com/andreasoprani/infer-ts).

## The problem

First of all, a little bit of context:

I work on [Scops.ai](https://scops.ai), a platform for intelligent predictive maintenance and energy monitoring.
One of the core functionalities of Scops is the analysis of time-series data coming from industrial machinery.
Our users can provide their data in various ways: IoT sensors, DB connections, APIs, manual insertion and, finally, through the upload of CSV/XLSX files (we'll focus on this now).

These files typically look something like this (apologies to non-US eyes):

```
| timestamp           | value |
| ------------------- | ----- |
| 01/01/2026 00:00:00 | 1     |
| 01/02/2026 00:00:00 | 2     |
| ...                 | ...   |
| 01/13/2026 00:00:00 | 13    |
```

And we would ingest it with our good old friend [Polars](https://pola.rs).
Polars is great, we switched from pandas a long time ago and we never looked back, but on the timestamp format inference side it's pretty limited and I'll show you why:

The way you transform a `str` series to a `datetime` one in Polars is through the [`series.str.to_datetime(format=...)`](https://docs.pola.rs/api/python/stable/reference/expressions/api/polars.Expr.str.to_datetime.html) function. Here you have two options, either you pass a format (assuming you know one) or you pass `None` and let Polars infer it.

In our case, we don't have a pre-determined format. Our clients are pretty heterogeneous, they span across continents and backgrounds, they may be ISO-abiding citizens or complete anarchists with legacy systems that spit out crazy formats.

So, we will let it infer the format for us. That seems great, right? WRONG!

Polars inference is pretty _lazy_, and not in the good sense.
If you try to use it, you will see that it does not use the whole series to solve ambiguities: it locks onto a format early and then parses the rest of the series with that. Here's an example using the data above:

```sh
uv run --python 3.14 --with 'polars==1.43.2' python - <<'PY'
import polars as pl

df = pl.DataFrame(
    {
        "timestamp": [
            "01/01/2026 00:00:00",
            "01/02/2026 00:00:00",
            "01/13/2026 00:00:00",
        ],
        "value": [1, 2, 13],
    }
)

print(df.with_columns(pl.col("timestamp").str.to_datetime()))
# raises InvalidOperationError: conversion from `str` to `datetime[μs]` failed in column 'timestamp'
# for 1 out of 3 values: ["01/13/2026 00:00:00"]
PY
```

Ok, then we'll pass it a format ourselves, but this means essentially opening up Pandora's box.
In a perfect world, as a society, we would have determined a standard (like ISO 8601) and everyone would stick to it. But this is not a perfect world and people use a variety of different formats to represent dates and times and we must write code that reliably gets the correct format unaided.
How do we do that?

## The solutions

So, what should our parser do when it encounters something like this?
Well, we have a few options:

### 1. Ask the user

Asking the user how their data is encoded at first seems like a sensible approach. They should know, right?

But it has some issues.

First, maybe they don't. Our users are mostly non-technical, or at least not in this area of expertise. They are usually energy managers, maintenance technicians, plant operators, and other people you can find in a factory doing real work (unlike us). They know how to run a production line but not necessarily the difference between `MM`, `mm` and `MMM`.

Second, even if they know, we are just moving the complexity from us to the user, which is not very nice. Ideally, if we can accomplish a task, we would like to do it ourselves and keep the product as easy and straightforward to use as possible (see [Tesler's Law](https://en.wikipedia.org/wiki/Law_of_conservation_of_complexity)).

I think in general this is not an unsolvable problem. So, let's try to do it ourselves.

### 2. Try a bunch of formats

A workaround that we used for a (long) while, is to keep a list of possible formats and to try converting the column with each one of them sequentially, if one is accepted we use that, if we fail with all of them we raise an error and the user must fix their file or contact us to go on. Every time a user had a format that we didn't accept yet, we could consider adding it to the list.

We also made it a bit composable, and in the end it looked something like this:

```py
def cast_datetime(df: pl.DataFrame, col: str) -> pl.DataFrame:
  datepart_formats = [
    "%Y-%m-%d",
    "%d/%m/%Y",
    "%Y%m%d",
    ...
  ]

  separators = [" ", "T"]

  timepart_formats = [
    "%H:%M:%S",
    "%H:%M",
    "%H:%M %Z",
    ...
  ]

  formats = datepart_formats + [
    f"{datepart}{separator}{timepart}"
    for datepart in datepart_formats
    for separator in separators
    for timepart in timepart_formats
  ]

  for fmt in formats:
    try:
      return df.with_columns(pl.col(col).str.to_datetime(fmt).alias(col))
    except Exception:
      pass

    # Some fallback here

  raise ValueError("No matching datetime format found")
```

This seems fine right? Well, it is _fine_.

It is not _great_ though.

The performance, for example, is not ideal. Even though this is Python-land and performance isn't always the first concern, this is not the kind of loop you want around a large series.

It would also need additional work to properly handle series that are fully ambiguous.

But it worked for a long time and it would still work today if we didn't change it (that's why we kept it for a long time).

But more importantly and philosophically it has a much bigger issue: it's ugly AF; or to be more PC it doesn't spark joy. We are essentially guessing formats until we find the correct one. We could do much better.

### 3. Write your own inference

Ideally, what I wanted to do was to use the whole column to infer the format.
Let's focus on the above example:

```
| timestamp           | value |
| ------------------- | ----- |
| 01/01/2026 00:00:00 | 1     |
| 01/02/2026 00:00:00 | 2     |
| ...                 | ...   |
| 01/13/2026 00:00:00 | 13    |
```

Here, a human would understand quite easily that the format is `MM/DD/YYYY HH:mm:ss`, but to do that you need to parse the whole column keeping in mind the formats that match it until you finally find a timestamp that disambiguates it.

Some of you might recognise that this is a pretty simple [Constraint Satisfaction Problem (or CSP)](https://en.wikipedia.org/wiki/Constraint_satisfaction_problem).

A CSP is a kind of logic problem where we have a space (a set of variables) for which we have to find a state (the values assigned to the variables) that satisfies a bunch of constraints.

Sudoku is a very famous example of a CSP: the initial provided values and the relationship between the various cells represents the constraints (i.e. no repetitions in a row, column or sub-grid) and the final values configuration found is a state that satisfies them.

In our case, each element of the series is a constraint that limits the number of compatible candidate formats and the final set of formats found after we have checked the whole series is the state that satisfies all the constraints. As I was saying, this is a pretty simple CSP, as we have essentially one variable (the final format) and the constraints are pretty limited in complexity. So we probably didn't need some fancy CSP solver, just a simple iteration on the list with incompatible candidates removal.

Consider the above example, you can see how this would work:

```
01/01/2026 → compatible with both DD/MM/YYYY and MM/DD/YYYY
01/02/2026 → still compatible with both
01/13/2026 → DD/MM/YYYY eliminated, only MM/DD/YYYY remains
```

I also wanted my implementation to be sounder, catching fully ambiguous series and reporting all compatible formats or erroring out when no format matches it.

Finally, I wanted it to be efficient, and that probably meant not doing it in pure Python. A lot of Python libraries and tools nowadays are C or Rust libraries with Python bindings (e.g., Polars itself), and it's clear to see why this was a good option also here.

Unfortunately, at the time, it didn't feel worth it to work on something like that: this was not a big bottleneck for Scops so it was better to allocate my work time on something else, and my free time was better spent doing something else (coding some small games or just afk).
So I shelved it.

But then, a few months ago, something changed.

## Enter coding agents

You might have noticed something changed in the development world towards the end of 2025 and start of 2026: coding agents suddenly were everywhere. Before that time, I was using mainly tab-completion tools (e.g. GitHub Copilot) and I was pretty happy with them, but then obviously the FOMO got me and I gave agents a go.

I realized that the project I had set aside a few years earlier was a perfect candidate for testing agents:

- it was a greenfield, self-contained project with no legacy code or functionality to maintain.
- it was relatively small, so I could easily review the output.
- I had the whole structure already in mind, so I could give precise guidance.
- it was very testable, allowing the agent to test stuff on its own without my direct feedback.
- it had a good dose of boilerplate involved that I didn't want to handle, i.e. the Python-Rust interoperability layer.

So I gave it a try in my free time, first with Amp and then, when I had no free tokens left, with Claude.

A few sessions of (very guided) _vibe-coding_ later, this is what I got:

## What infer-ts does

Infer-ts can (as the name suggests) infer the timestamp format(s) of any Python `str` (`| None`) iterable, including a Polars series.
So, all of these will work:

```py
import polars as pl
import infer_ts

l = [
  "01/01/2026 00:00:00",
  "01/02/2026 00:00:00",
  "01/13/2026 00:00:00",
]

# List
infer_ts.infer_format(l) # ['%m/%d/%Y %H:%M:%S']

# Iterable
infer_ts.infer_format((v for v in l)) # ['%m/%d/%Y %H:%M:%S']

# pl.Series
infer_ts.infer_format(pl.Series("timestamp", l)) # ['%m/%d/%Y %H:%M:%S']
```

It handles ambiguous formats by returning all of them:

```py
ambiguous = l[:2]

infer_ts.infer_format(ambiguous) # ['%d/%m/%Y %H:%M:%S', '%m/%d/%Y %H:%M:%S']
```

By default, inference stops as soon as only one candidate remains. If we also want to validate the inferred format against the entire input, we can enable exhaustive mode:

```py
values = l + ["not a timestamp"]

# Stops as soon as only one candidate format remains
infer_ts.infer_format(values) # ['%m/%d/%Y %H:%M:%S']

# Checks that the inferred format is valid for every value
infer_ts.infer_format(values, exhaustive=True) # []
```

And, finally, it can directly convert Polars series and expressions:

```py
series = pl.Series("timestamp", l)

infer_ts.to_datetime(series)
# shape: (3,)
# Series: 'timestamp' [datetime[μs]]
# [
#     2026-01-01 00:00:00
#     2026-01-02 00:00:00
#     2026-01-13 00:00:00
# ]

df = pl.DataFrame({"timestamp": l})

# Column name

df.with_columns(infer_ts.to_datetime("timestamp"))
# shape: (3, 1)
# ┌─────────────────────┐
# │ timestamp           │
# │ ---                 │
# │ datetime[μs]        │
# ╞═════════════════════╡
# │ 2026-01-01 00:00:00 │
# │ 2026-01-02 00:00:00 │
# │ 2026-01-13 00:00:00 │
# └─────────────────────┘

# Polars expression + time_unit specified

df.with_columns(infer_ts.to_datetime(pl.col("timestamp"), time_unit="ms"))
# shape: (3, 1)
# ┌─────────────────────┐
# │ timestamp           │
# │ ---                 │
# │ datetime[ms]        │
# ╞═════════════════════╡
# │ 2026-01-01 00:00:00 │
# │ 2026-01-02 00:00:00 │
# │ 2026-01-13 00:00:00 │
# └─────────────────────┘


# Resolve an ambiguous slash date using a preference

ambiguous_df = pl.DataFrame({"timestamp": ["01/02/2026", "03/04/2026"]})
ambiguous_df.with_columns(
  infer_ts.to_datetime(
    "timestamp",
    raise_on_multiple=False,
    date_preference="us",
  )
)
# shape: (2, 1)
# ┌─────────────────────┐
# │ timestamp           │
# │ ---                 │
# │ datetime[μs]        │
# ╞═════════════════════╡
# │ 2026-01-02 00:00:00 │
# │ 2026-03-04 00:00:00 │
# └─────────────────────┘
```

It supports a variety of formats:

```py
# ISO datetime
infer_ts.infer_format(["2026-01-13T14:30:00"])
# ['%Y-%m-%dT%H:%M:%S']

# Month name and 12-hour time
infer_ts.infer_format(["Jan 13, 2026 02:30 PM"])
# ['%b %d, %Y %I:%M %p']

# Date only
infer_ts.infer_format(["13.01.2026"])
# ['%d.%m.%Y']

# Timezone offset
infer_ts.infer_format(["2026-01-13T14:30:00+01:00"])
# ['%Y-%m-%dT%H:%M:%S%:z']

# Unix timestamp
infer_ts.infer_format(["1768314600"])
# ['@unix_seconds']

# Compact datetime
infer_ts.infer_format(["20260113T143000"])
# ['%Y%m%dT%H%M%S']
```

...and many more, please open a PR if some weird format you use is not supported

## How does this work internally?

---

## Internal algorithm

- Incremental inference state machine:
  - first non-null value initializes candidates;
  - subsequent values retain only compatible candidates;
  - early exit when one candidate remains, unless exhaustive mode is enabled;
  - final result returns all surviving candidates.
- Nulls and empty strings are skipped.
- Possible outcomes:
  - no data;
  - no match;
  - one format;
  - multiple formats.
- Performance angle:
  - resolves early in many real datasets;
  - exhaustive mode validates the full column.

## Compositional format design

- Formats are not represented as a flat list.
- Structure:
  - `Date`;
  - `DateTime = Date + Separator + Time + Timezone?`;
  - `Unix`.
- Benefits:
  - easier to add new variants;
  - combinations happen naturally;
  - invalid combinations are eliminated by validation;
  - less manual enumeration.

## Polars, Rust, and PyO3

- Python-facing API, Rust core.
- Polars expression plugin architecture.
- High-level flow:
  - Python API;
  - Polars expression plugin;
  - Rust inference engine;
  - Polars native cast.
- Why Rust is interesting for Python developers:
  - performance;
  - safety;
  - good fit with Polars internals;
  - PyO3/maturin make packaging feasible.
- Mention resources:
  - Marco Gorelli's Polars plugin tutorial;
  - cookiecutter-polars-plugins;
  - previous Python Milano Rust/PyO3 talks.
- Note mistake/lesson:
  - should have looked at Gorelli's plugin resources earlier.

## Less glamorous problems

- Python/Rust packaging.
- Type checking and stubs.
- Runtime Polars namespace API:
  - `pl.col("ts").infer_ts.to_datetime()`;
  - pyright/static typing friction.
- API decision:
  - namespace API vs unified `infer_ts.to_datetime(col)`.
- Performance:
  - inference pass + Polars cast pass;
  - future single-pass infer+cast may be possible but non-trivial.
- Real production edge case after first use.
- Maintenance cost after "just building" the thing.

## How it was built with coding agents

- Project almost entirely written with LLM assistance.
- Important distinction:
  - code was generated by agents;
  - the project was not designed by agents.
- Human responsibilities:
  - problem framing;
  - scope;
  - API design;
  - architecture;
  - trade-offs;
  - review;
  - deciding what not to build.

## README, TODO, and conversation as design doc

- No single formal design document.
- Design emerged through:
  - README;
  - TODO;
  - conversations with agents;
  - continuous review;
  - course corrections.
- Guardrails against:
  - "AI builds features, not architecture";
  - "velocity illusion widens your scope".
- README/TODO as scope control.
- Conversation as maieutic design process.

## Steering and taste

- Key skill was not prompting for code, but steering.
- Examples of steering decisions:
  - API does not feel right;
  - abstraction is too complicated;
  - edge case must be explicit;
  - feature belongs in TODO;
  - code needs to be understood line by line.
- Link to "I'm going back to writing code by hand".
- Compare briefly with antirez/ds4:
  - similar in heavy AI involvement;
  - different because important code was reviewed and understood.

## Takeaways

- Build small tools:
  - useful;
  - fun;
  - educational;
  - real enough to force real decisions.
- Agents lower the cost of starting, not the cost of understanding.
- Software engineering still depends on:
  - architecture;
  - design;
  - taste;
  - responsibility.
- "You can just do stuff" is true, but:
  - details still matter;
  - scope creep becomes easier;
  - maintenance remains;
  - bugs and users still exist.
- The cost does not disappear; it moves.

## Sources and links

- [`infer-ts` on GitHub](https://github.com/andreasoprani/infer-ts)
- [Marco Gorelli — Polars plugins tutorial](https://marcogorelli.github.io/polars-plugins-tutorial/)
- [cookiecutter-polars-plugins](https://github.com/MarcoGorelli/cookiecutter-polars-plugins)
- [I'm going back to writing code by hand](https://blog.k10s.dev/im-going-back-to-writing-code-by-hand/)
- [DS4 by antirez](https://github.com/antirez/ds4)
