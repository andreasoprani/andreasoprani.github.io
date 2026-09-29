+++
title = "infer-ts: on small projects and AI coding."
description = ""
date = 2026-09-29
[extra]
unlisted = true
+++

# infer-ts: on small projects and AI coding.

## A disclaimer

This post doesn't have a single clear purpose, it is the textual version of a talk (not presented yet) that discusses multiple thoughts that came to me during and after the development of a small library that I needed for work.

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

| timestamp           | value |
| ------------------- | ----- |
| 01/01/2026 00:00:00 | 1     |
| 01/02/2026 00:00:00 | 2     |
| ...                 | ...   |
| 01/13/2026 00:00:00 | 13    |

We ingest these files with our good old friend [Polars](https://pola.rs).
Polars is great, we switched from pandas a long time ago and we never looked back, but on the timestamp format inference side it's pretty limited and I'll show you why:

The way you transform a `str` series to a `datetime` one in Polars is through the [`series.str.to_datetime(format=...)`](https://docs.pola.rs/api/python/stable/reference/expressions/api/polars.Expr.str.to_datetime.html) function. Here you have two options, either you pass a format (assuming you know one) or you pass `None` and let Polars infer it.

In our case, we don't have a pre-determined format. Our clients are pretty heterogeneous, they span across continents and backgrounds, they may be ISO-abiding citizens or complete anarchists with legacy systems that spit out crazy formats.

So, we will let it infer the format for us. That seems great, right? WRONG!

Polars inference is pretty _lazy_, and not in the good sense.
It does not use the whole series to resolve ambiguity: it locks onto a format early and then parses the rest of the series with that. Here's an example using the data above:

```py
import polars as pl # v1.43.2

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

```

The last row rules out day-first dates, but Polars has already committed to that interpretation.

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

But it worked, and it would still work today. That’s why it stayed around.

But more importantly and philosophically it has a much bigger issue: it's ugly AF; or to be more PC it doesn't spark joy. We are essentially guessing formats until we find the correct one. We could do much better.

### 3. Write your own inference

Ideally, what I wanted to do was to use the whole column to infer the format.
Let's focus on the above example:

| timestamp           | value |
| ------------------- | ----- |
| 01/01/2026 00:00:00 | 1     |
| 01/02/2026 00:00:00 | 2     |
| ...                 | ...   |
| 01/13/2026 00:00:00 | 13    |

Here, a human would understand quite easily that the format is `MM/DD/YYYY HH:mm:ss`, but to do that you need to parse the whole column keeping in mind the formats that match it until you finally find a timestamp that disambiguates it.

Some of you might recognise that this is a pretty simple [Constraint Satisfaction Problem (or CSP)](https://en.wikipedia.org/wiki/Constraint_satisfaction_problem).

A CSP is a kind of logic problem where we have a space (a set of variables) for which we have to find a state (the values assigned to the variables) that satisfies a bunch of constraints.

Sudoku is a very famous example of a CSP: the initial provided values and the relationship between the various cells represents the constraints (i.e. no repetitions in a row, column or sub-grid) and the final values configuration found is a state that satisfies them.

In our case, we have one variable, the timestamp format, and each element of the series adds a constraint on its possible values. After checking the whole series, the surviving formats are all the possible assignments that satisfy those constraints. With only one variable and straightforward constraints, we don’t need a fancy solver. We just need to progressively eliminate incompatible candidates.

Consider the above example, you can see how this would work:

```
01/01/2026 → compatible with both DD/MM/YYYY and MM/DD/YYYY
01/02/2026 → still compatible with both
01/13/2026 → DD/MM/YYYY eliminated, only MM/DD/YYYY remains
```

I also wanted my implementation to be sounder, catching fully ambiguous series and reporting all compatible formats or erroring out when no format matches it.

Finally, I wanted it to be efficient, and that probably meant not doing it in pure Python. A lot of Python libraries and tools nowadays are C or Rust libraries with Python bindings (e.g., Polars itself), and it's clear to see why this was a good option also here.

Unfortunately, at the time, it didn't feel worth it to work on something like that: this was not a big bottleneck for Scops so it was better to allocate my work time on something else, and I wanted to spend my free time in other ways (coding some [small games](https://sprn.it/blog/mwpc) or just afk).
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

timestamps = [
  "01/01/2026 00:00:00",
  "01/02/2026 00:00:00",
  "01/13/2026 00:00:00",
]

# List
infer_ts.infer_format(timestamps) # ['%m/%d/%Y %H:%M:%S']

# Iterable
infer_ts.infer_format((v for v in timestamps)) # ['%m/%d/%Y %H:%M:%S']

# pl.Series
infer_ts.infer_format(pl.Series("timestamp", timestamps)) # ['%m/%d/%Y %H:%M:%S']
```

It handles ambiguous formats by returning all of them:

```py
ambiguous = timestamps[:2]

infer_ts.infer_format(ambiguous) # ['%d/%m/%Y %H:%M:%S', '%m/%d/%Y %H:%M:%S']
```

By default, inference stops as soon as only one candidate remains. If we also want to validate the inferred format against the entire input, we can enable exhaustive mode:

```py
values = timestamps + ["not a timestamp"]

# Stops early
infer_ts.infer_format(values) # ['%m/%d/%Y %H:%M:%S']

# Checks until the end
infer_ts.infer_format(values, exhaustive=True) # []
```

### Conversion

A Series can be converted directly:

```py
series = pl.Series("timestamp", timestamps)

infer_ts.to_datetime(series)
# shape: (3,)
# Series: 'timestamp' [datetime[μs]]
# [
#     2026-01-01 00:00:00
#     2026-01-02 00:00:00
#     2026-01-13 00:00:00
# ]
```

Inside a DataFrame, we can pass a column name:

```py
df = pl.DataFrame({"timestamp": timestamps})

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

```

Or a Polars expression, optionally specifying the output time unit:

```py
df.with_columns(infer_ts.to_datetime(pl.col("timestamp"), time_unit="ms"))
# Same values, with dtype datetime[ms]
```

When dates remain ambiguous, conversion raises an error by default.
We can explicitly allow a choice and set a preference:

```py
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

### Formats, parsing, and inference

Let's start from the formats. Each candidate is represented by one of three variants, with datetime formats composed from smaller components:

```text
Format
├── Date(DateFormat)
│   └── DateFmt — ISO, EU/US slash dates, etc.
├── DateTime(DateTimeFormat)
│   ├── DateFmt
│   └── TimeComponent
│       ├── Separator — T or space
│       ├── TimeFmt — HH:MM, HH:MM:SS, fractional seconds, etc.
│       ├── Optional timezone — Z, ±HH:MM, or ±HHMM
│       └── spaced_tz — whether a space precedes the timezone
└── Unix(UnixFormat)
    └── UnixPrecision — seconds, milliseconds, microseconds, or nanoseconds
```

As you can see, similarly to our python-based solution, the datetime format takes a combinatorial approach so that any new format can be added without much new complexity.

Each format implements:

- A parser that returns all the possible formats compatible with an input string.
- A validator that given a format and a string says if that specific format is compatible with the string.
- A function that returns the string representation of the format accepted by Polars.

Having built this structure, the parsing is then pretty simple, as we can let each subcomponent do its own part isolated from the rest. The first non-empty value of the series is fed to the parser, which finds all the compatible formats:

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

The benefit of this approach is that inference can stop early for most series. When converting, we still parse the entire column and, by default, raise if any non-blank value fails. So we don't need exhaustive inference followed by another full parsing pass just to check the input twice.

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

For packaging everything, I used [maturin](https://github.com/pyo3/maturin), which is the standard PyO3 way of compiling Rust libraries and bundling them with the Python wrappers into a wheel.

If you want to explore this space I suggest you look into [PyO3](https://pyo3.rs), [`pyo3-polars`](https://github.com/pola-rs/polars/tree/main/pyo3-polars) and particularly into [Marco Gorelli's tutorial on Polars plugins](https://marcogorelli.github.io/polars-plugins-tutorial/). Unfortunately I found out about this guide after having implemented the library, but you can avoid making the same mistake. Same goes for his [cookiecutter template for Polars plugins](https://github.com/MarcoGorelli/cookiecutter-polars-plugins/tree/main).

### TODOs

This is by no means a finished project, it's currently at version 0.2.0 and I still have some TODOs I would like to tackle (even though I don't know if I ever will).

#### 1. Polars namespace

I didn't mention this before in the examples but the library also registers infer_ts in the Polars expression namespace, i.e. allowing to do something like `pl.col("ts").infer_ts.to_datetime()`. This is cool but it poses some problems and adds little use: the main issue is that this registration is done at runtime, so static type checkers like `pyright` or `ty` (that I like to use) are unable to understand what's going on, unless the user explicitly configures custom stubs in their codebase.
Since the namespace offers little over the function API described above, I am considering removing it.

#### 2. Single-pass conversion

The `infer_ts.to_datetime(col)` conversion right now is basically just a QoL improvement for the user, since it just wraps the inference, calls the Polars conversion with the inferred format directly and checks for failed conversions.
It could be interesting in the future to explore the possibility of doing inference and conversion in a single pass, but doing that naively would be probably slower so I would need to investigate how this is done inside Polars and how to do various performance improvements.

Furthermore, the inference right now adds little overhead in non-exhaustive mode, which is what you would do anyway when converting the series immediately afterward.
Here's a benchmark I did on my laptop, with a series of 1-minute spaced timestamps for a whole year (525600 rows) with an ambiguous format for the first 12 days (%m/%d/%Y %H:%M:%S, 17280 rows), testing direct Polars conversion when the format is known (so the second step of our conversion) against `infer_ts.to_datetime(series)`:

| Conversion                     |       Median |   Overhead |
| ------------------------------ | -----------: | ---------: |
| Polars, known format           | **21.82 ms** |          — |
| `infer_ts.to_datetime(series)` | **25.54 ms** | **+17.0%** |

## How did it go?

### (Not) vibe-coded

As I said at the beginning, almost every line of code in this library was generated by an LLM, but it probably can't be defined as vibe-coded as I had a very active approach in the development.

I worked on infer-ts in February 2026. At the time I was probably working with GPT-5.3 and Claude Opus 4.6. Now some time has passed and the recent models are far superior to those ones, but I wouldn't say that my way of working with them has changed much. With every new model I keep seeing that, if left completely free to write what they want, they produce code that works but is very ugly and would require a lot of refactoring effort to maintain and expand, effort that an experienced programmer would simply put in writing the code the first time.

Since this was a greenfield project I was able to just provide plans for how I wanted things to work without worrying about compatibility with existing code. I relied heavily on discussions with the LLM to develop and refine a plan, a kind of _maieutic_ approach to software development. I recorded the plan in `TODO.md` and the overall direction in `README.md`. Even then, I had to review every change and request many adjustments.

One example is the `Format` structure and specifically the `DateTimeFormat` one: when coding a PoC of the parser, the agent started immediately adding full datetime formats (like the ISO 8601 one) and their parser. This seemed ok for a PoC but when asked to add support for other formats it just proceeded to implement their parser, without considering that this would lead to a combinatorial explosion when we need to consider different date-time separators, different timezone formats and different combinations of date and time formats. In that case I had to specifically suggest composing formats from date, time, separator, and timezone components, otherwise it would have continued to add new formats unfazed.

I believe this happens because the model doesn't experience friction like you and me do, it is just tasked to add the new format and it does so. Adding a few extra functions for its parsing and validation is not a huge deal, that's what the code does for each format anyway.

This usually results also in scope creep. The agents kept suggesting to add new features, it made sense to them, this is a library, you want to provide features. But every new feature adds complexity to the codebase and it's stuff that you have to maintain. So you have to provide the _taste_ to say no to some (most) of them.

### In production

At some point I was happy with the result, I had a lot of tests to check different formats and I added all the ones I wanted to support for a first version, so I decided to release this and start using it in Scops.
I tested the file import with real data and I immediately noticed an issue: the data was being saved but the timestamps were wrong. This happened because we were correctly detecting that a tz was being used but that info was not used when converting the expression to datetime, my big pile of AI-written tests didn't catch that. So I immediately fixed this and released version 0.1.2.

After a few months an upload from a client was rejected, he was using a format like `1/13/26 21:00`.
This was an easier fix, and I used this opportunity to add a bunch of new formats in version 0.1.3.

Funny enough, writing this post led to a few more changes, which I released as version 0.2.0. I'll leave those for the curious reader to explore.

## Conclusions

So, here we are at the end of our journey.

I don't have many technical takeaways to give here, I think the project is not that complex and at this point you can probably see for yourself why it made sense to develop it.

I feel that now it's a great moment to develop small, useful tools that you (and maybe only you) need like this one. LLMs have lowered the cost of these kinds of projects (especially contained, green-field ones).

This reduction in cost, at least in my case, is mainly due to absence of friction. I usually don't want to spend my free time understanding how some boilerplate library works, so I typically procrastinate it or just shelve the project, but if there's an LLM doing that for you, then you can focus on the core functionalities.

This might sound like a naïve hopeful view but what this project has shown me (and what I see daily at work) is that for now our jobs are safe, as I cannot let LLMs make changes unchecked. Maybe in the future it will be the case, but that seems hard to me: what I keep seeing is that since the LLMs don't have to live with the consequences of their code, they don't experience any inherent push to make the future work easier. They just keep producing a growing pile of code that is destined to break under its own weight at some point.

Some time ago, people started saying that _taste_ is the differentiator between humans and LLMs. This is now kind of a buzzword, but I think it sums it up quite nicely. Taste is essentially the result of our experience, of seeing what worked and what didn't in the past, of living with and being forced to work on a project that has many flaws, and the delight of making the changes needed to fix it. LLMs are spawned into existence when asked to implement a feature and they die when they're done, leaving their mess to the next one (and to you), they don't have this burden (and luxury).

Anyways, this has been fun, I guess you can just do stuff.
