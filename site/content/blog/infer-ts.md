+++
title = "infer-ts"
description = ""
date = 2026-08-14
[extra]
unlisted = true
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

The performance, for example, is not ideal. Even though this is Python-land and performance isn't always the first concern, you typically don't want this kind of guessing loop around a large series.

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

In our case, we have one variable, the timestamp format, and each element of the series adds a constraint on its possible values. After checking the whole series, the surviving formats are all the possible assignments that satisfy those constraints. As I was saying, this is a pretty simple CSP, as we have only one variable and the constraints are pretty limited in complexity. So we probably didn't need some fancy CSP solver, just a simple iteration on the list with incompatible candidates removal.

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

### Format inference

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

### Conversion

It can also directly convert Polars series and expressions:

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

### Supported formats

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

### The inference

Let's start from the formats, the structure is pretty simple:

```rs
pub enum DateFmt {
    Iso, // `YYYY-MM-DD` (ISO 8601)
    SlashEU, // `DD/MM/YYYY` (EU convention)
    SlashUS, // `MM/DD/YYYY` (US convention)
    ...
}

pub enum Separator {
    T, // `T` or `t` (ISO 8601)
    Space, // Single space
}

pub enum TimeFmt {
    Hm, // `HH:MM`
    Hms, // `HH:MM:SS`
    HmsFrac, // `HH:MM:SS.f` (1-9 fractional digits)
    ...
}

pub enum Timezone {
    Utc, // `Z` or `z` (UTC)
    Offset, // `±HH:MM` (with colon)
    OffsetCompact, // `±HHMM` (compact, no colon)
}

pub enum UnixPrecision {
    Seconds, // 9–10 digits (seconds since epoch, ~1973–2286)
    Milliseconds,
    ...
}

pub struct DateFormat {
    pub date: DateFmt,
}

pub struct TimeComponent {
    pub separator: Separator,
    pub format: TimeFmt,
    pub timezone: Option<Timezone>,
    pub spaced_tz: bool,
}

pub struct DateTimeFormat {
    pub date: DateFmt,
    pub time: TimeComponent,
}

pub struct UnixFormat {
    pub precision: UnixPrecision,
}

pub enum Format {
    Date(DateFormat),
    DateTime(DateTimeFormat),
    Unix(UnixFormat),
}
```

As you can see, similarly to our python-based solution, the datetime format takes a combinatorial approach so that any new format can be added without much new complexity.

Each format implements:

- A parser that returns all the possible formats compatible with an input string.
- A validator that given a format and a string says if that specific format is compatible with the string.
- A function that returns the string representation of the format accepted by Polars.

Having built this structure, the parsing is then pretty simple, as we can let each subcomponent do its own part isolated from the rest. The first non-empty value is fed to the parser, which finds all the compatible formats:

```text
Format::parse(value)
├── DateFormat::parse(value)
│   └── For each DateFmt: parse_date(value)
│       └── Keep the match only if nothing remains (date-only)
├── DateTimeFormat::parse(value)
│   ├── For each DateFmt: parse_date(value)
│   └── parse_time_components(remaining, date_fmt, matches)
│       ├── parse_separator(remaining, sep)
│       ├── parse_time(after_sep, time_fmt)
│       └── parse_timezone(tz_input, tz) — if a suffix remains
│           └── Keep complete matches with no unconsumed input
└── UnixFormat::parse(value)
    └── For each UnixPrecision: UnixFormat::validates(value)

→ Collect all matches as Format::Date, Format::DateTime, or Format::Unix
```

Then, the subsequent values are fed to the validators of the surviving formats until a terminating condition is reached (either the series is exhausted, no format survived or just one remains and the user requested a non-exhaustive inference).

The benefit about this approach is that it doesn't sacrifice robustness but, at the same time, it will resolve early for most series (provided that a non-exhaustive inference is satisfying for the user, which it usually is when the series is then converted entirely and any error will be raised anyway).

### The Python interface

As you can see from the example above, the library has two ways in which it can be used:

- Simply by inferring the format of a Polars series or an iterable with `infer_ts.infer_format(series)`.
- Directly converting a `str` Polars expression to a `datetime` Polars expression through `infer_ts.to_datetime(pl.col("series_name"))`.

These use two different mechanisms. PyO3 handles Python-Rust interoperability, while `pyo3-polars`, part of the Polars ecosystem, adds support for Polars data types and expression plugins. In the first case, we use its PySeries wrapper to pass a Python Polars Series directly to a Rust function exposed through PyO3. In the second, we register a Rust function as a plugin that Polars executes natively when evaluating an expression.

The implementation of this is actually quite straightforward, thanks to `pyo3-polars`, but it's still helpful to have an LLM deal with this boilerplate. In our case it looks something like this:

```rs
// Direct Python call through PyO3
#[pyfunction]
#[pyo3(signature = (series, exhaustive=false))]
fn infer_format_series(
    series: PySeries,
    exhaustive: bool,
) -> PyResult<Vec<String>> {
    // Access the Rust Series, run inference, return format strings.
    // ...
}

// Native execution through a Polars expression plugin
#[polars_expr(output_type_func=to_datetime_output_us)]
fn to_datetime_expr_us(
    inputs: &[Series],
    kwargs: ToDatetimeKwargs,
) -> PolarsResult<Series> {
    to_datetime_impl(inputs, kwargs, TimeUnit::Microseconds)
}
```

The first function can be called directly from Python, while the second one has to be registered to be callable by Polars when evaluating an expression:

```py
register_plugin_function(
    plugin_path=PLUGIN_PATH,
    function_name=f"to_datetime_expr_{time_unit}",
    args=[expr],
    is_elementwise=False,
    kwargs={...},
)
```

For packaging everything, I used [maturin](https://github.com/pyo3/maturin), which is a build tool in the PyO3 ecosystem that compiles Rust libraries and bundles them with the Python wrappers into a wheel.

If you want to explore this space I suggest you look into [PyO3](https://pyo3.rs), [`pyo3-polars`](https://github.com/pola-rs/polars/tree/main/pyo3-polars) and particularly into [Marco Gorelli's tutorial on Polars plugins](https://marcogorelli.github.io/polars-plugins-tutorial/). Unfortunately I found out about this guide after having implemented the library, but you can avoid making the same mistake. Same goes for his [cookiecutter template for Polars plugins](https://github.com/MarcoGorelli/cookiecutter-polars-plugins/tree/main).

### TODOs

This is by no means a finished project, it's currently at version 0.1.3 (we'll see later why) and I still have some TODOs I would like to tackle (even though I don't know if I ever will).

#### 1. Polars namespace

I didn't mention this before in the examples but the library also registers infer_ts in the Polars expression namespace, i.e. allowing to do something like `pl.col("ts").infer_ts.to_datetime()`. This is cool but it poses some problems and adds little use: the main issue is that this registration is done at runtime, so static type checkers like `pyright` or `ty` (that I like to use) are unable to understand what's going on, unless the user explicitly configures custom stubs in their codebase.
Since the namespace offers little over the function API described above, I am considering removing it

#### 2. Single-pass conversion

The `infer_ts.to_datetime(col)` conversion right now is basically just a QoL improvement for the user, since it just wraps the inference and calls the Polars conversion with the inferred format directly.
It could be interesting in the future to explore the possibility of doing inference and conversion into a single pass, but doing that naively would be probably slower so I would need to investigate how this is done inside Polars and how to do various performance improvements.

Furthermore, the inference right now adds little overhead in non-exhaustive mode, which is what you would do anyway when converting the series immediately afterward.
Here's a benchmark I did on my laptop, with a series of 1-minute spaced timestamps for a whole year (525600 rows) with an ambiguous format for the first 12 days (%m/%d/%Y %H:%M:%S, 17280 rows), testing direct Polars conversion when the format is known (so the second step of our conversion) against `infer_ts.to_datetime(series)`:

| Conversion                                      |       Median |
| ----------------------------------------------- | -----------: |
| Polars, known format                            | **21.28 ms** |
| `infer_ts.to_datetime(series)`                  | **23.34 ms** |
| `infer_ts.to_datetime(series, exhaustive=True)` | **34.23 ms** |

## How did it go?

### (Not) vibe-coded

As I said at the beginning, almost every line of code in this library was generated by an LLM, but it probably can't be defined as vibe-coded as I had a very active approach in the development.

I worked on infer-ts in February 2026. At the time I was probably working with GPT-5.3 and Claude Opus 4.6. Now some time has passed and the recent models are far superior to those ones, but I wouldn't say that my way of working with them has changed much. With every new model I keep seeing that, if left completely free to write what they want, they produce code that works but is very ugly and would require a lot of refactoring effort to maintain and expand, effort that an experienced programmer would simply put in writing the code the first time.

Since this was a greenfield project I was able to just provide plans for how I wanted things to work without worrying about compatibility with existing code. I relied heavily on discussions with the LLM to come up with a plan and refine it (kind of a _maieutic_ approach to software development), writing the agreed plan in the TODO.md file and documenting what the direction was in the README.md file, and I still had to review every single change the agent did and request many adjustments.

One example is the `Format` structure and specifically the `DateTimeFormat` one: when coding a PoC of the parser, the agent started immediately adding full datetime formats (like the ISO 8601 one) and their parser. This seemed ok for a PoC but when asked to add support for other formats it just proceeded to implement their parser, without considering that this would lead to a combinatorial explosion when we need to consider different date-time separators, different timezone formats and different combinations of date and time formats. In that case I had to specifically suggest composing formats from date, time, separator, and timezone components, otherwise it would have continued to add new formats unfazed.

I believe this happens because the model doesn't experience friction like you and me do, it is just tasked to add the new format and it does so. Adding a few extra functions for its parsing and validation is not a huge deal, that's what the code does for each format anyway.

This usually results also in scope creep. The agents kept suggesting to add new features, it made sense to them, this is a library, you want to provide features. But every new feature adds complexity to the codebase and it's stuff that you have to maintain. So you have to provide the _taste_ to say no to some (most) of them.

### In production

At some point I was happy with the result, I had a lot of tests to check different formats and I added all the ones I wanted to support for a first version, so I decided to release this and start using it in Scops.
I tested the file import with real data and I immediately noticed an issue: the data was being saved but the timestamps were wrong. This happened because we were correctly detecting that a tz was being used but that info was not used when converting the expression to datetime. So I immediately fixed this and released version 0.1.2.

After a few months an upload from a client was rejected, he was using a format like `1/13/26 21:00`.
This was an easier fix, and I used this opportunity to add a bunch of new formats in version 0.1.3.

## Takeaways

TODO: fix this and add takeaways about the technical aspects

I'm gonna repeat myself, but now you can also see for yourself that this post was a bit all over the place, so the takeaways are in that spirit too, here they are:

- I think that now is a great moment for developing small, useful tools that you (and maybe only you) need. Agent/vibe coding (or however you wanna call it) has lowered the cost of these projects.
- Part of the cost lowered, in my case, is due to friction. I usually don't want to spend my free time understanding how some boilerplate library works, so I typically procrastinate it or just shelve these kinds of projects, but if there's an LLM doing that for you, then you can focus on the core functionalities.
- These projects can be educational even if an LLM writes most of the code, provided that you are curious enough to try to understand what it's doing.
- Finally, I agree with the ones that say that writing the software was never the job of the developers. I believe that the job is taking the responsibility and the ownership of the code. We, as humans, have to live with the code months or years after having written it, experiencing the friction of adding new stuff or changing something, and I think that this leads to the acquisition of _taste_.
