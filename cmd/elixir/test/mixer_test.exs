defmodule MixerTest do
  use ExUnit.Case
  doctest Mixer

  test "greets the world" do
    assert Mixer.hello() == :world
  end
end
